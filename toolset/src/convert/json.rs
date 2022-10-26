use std::{
    collections::HashMap,
    io::{Read, Write},
};

use anyhow::{Context, Result};
use bdat::types::{Cell, ColumnDef, Label, RawTable, Row, ValueType};
use clap::Args;
use serde::{de::DeserializeSeed, Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{schema::FileSchema, BdatDeserialize, BdatSerialize, ConvertArgs};

#[derive(Args)]
pub struct JsonOptions {
    /// If this is set, JSON output will include spaces and newlines
    /// to improve readability.
    #[arg(long)]
    pretty: bool,
}

#[derive(Serialize, Deserialize)]
struct JsonTable {
    schema: Option<Vec<ColumnSchema>>,
    rows: Vec<TableRow>,
}

#[derive(Serialize, Deserialize)]
struct TableRow {
    #[serde(rename = "$id")]
    id: usize,
    #[serde(flatten)]
    cells: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize, Serialize)]
struct ColumnSchema {
    name: String,
    #[serde(rename = "type")]
    ty: ValueType,
    hashed: bool,
}

pub struct JsonConverter {
    untyped: bool,
    pretty: bool,
}

impl JsonConverter {
    pub fn new(args: &ConvertArgs) -> Self {
        Self {
            untyped: args.untyped,
            pretty: args.json_opts.pretty,
        }
    }

    fn convert(&self, bdat: bdat::types::Value) -> Value {
        serde_json::to_value(bdat).unwrap()
    }
}

impl BdatSerialize for JsonConverter {
    fn write_table(&self, table: RawTable, writer: &mut dyn Write) -> Result<()> {
        let schema = (!self.untyped).then(|| {
            table
                .columns
                .iter()
                .map(|c| ColumnSchema {
                    name: c.label.to_string(),
                    ty: c.ty,
                    hashed: matches!(c.label, Label::Unhashed(_)),
                })
                .collect::<Vec<_>>()
        });

        let rows = table
            .rows
            .into_iter()
            .map(|mut row| {
                let cells = table
                    .columns
                    .iter()
                    .map(|col| {
                        (
                            col.label.to_string(),
                            serde_json::to_value(&row.cells.remove(0)).unwrap(),
                        )
                    })
                    .collect();

                TableRow { id: row.id, cells }
            })
            .collect::<Vec<_>>();

        let json = JsonTable { schema, rows };
        if self.pretty {
            serde_json::to_writer_pretty(writer, &json)
        } else {
            serde_json::to_writer(writer, &json)
        }
        .context("Failed to write JSON")?;

        Ok(())
    }

    fn get_file_name(&self, table_name: &str) -> String {
        format!("{table_name}.json")
    }
}

impl BdatDeserialize for JsonConverter {
    fn read_table(
        &self,
        name: Option<Label>,
        schema: &FileSchema,
        reader: &mut dyn Read,
    ) -> Result<RawTable> {
        let table: JsonTable =
            serde_json::from_reader(reader).context("failed to read JSON table")?;

        let (columns, column_map, _): (Vec<ColumnDef>, HashMap<String, (usize, ValueType)>, _) =
            table
                .schema
                .expect("TODO, no column schema")
                .into_iter()
                .fold(
                    (Vec::new(), HashMap::default(), 0),
                    |(mut cols, mut map, idx), col| {
                        map.insert(col.name.clone(), (idx, col.ty));
                        cols.push(ColumnDef {
                            ty: col.ty,
                            label: Label::parse(col.name, col.hashed),
                            offset: 0, // only used when reading bdats
                        });
                        (cols, map, idx + 1)
                    },
                );

        let rows: Vec<_> = table
            .rows
            .into_iter()
            .map(|r| {
                let id = r.id;
                let mut cells = vec![None; r.cells.len()];
                for (k, v) in r.cells {
                    let (index, ty) = column_map[&k];
                    cells[index] = Some(ty.as_cell_seed().deserialize(v).unwrap());
                }
                let old_len = cells.len();
                let cells: Vec<Cell> = cells.into_iter().flatten().collect();
                if cells.len() != old_len {
                    panic!("rows must have all cells");
                }
                Row { id, cells }
            })
            .collect();

        Ok(RawTable {
            name,
            rows,
            columns,
        })
    }

    fn get_table_extension(&self) -> &'static str {
        "json"
    }
}
