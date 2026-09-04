//! CLI for the reusable durable relational engine.

use std::env;
use std::process::ExitCode;

use db_core::{DbError, Result};
use db_storage_log::relational::{Cell, Column, ColumnType, RelOp, RelationalEngine, Schema};

fn users_schema() -> Schema {
    Schema {
        columns: vec![
            Column {
                name: "id".to_owned(),
                ty: ColumnType::Int64,
            },
            Column {
                name: "name".to_owned(),
                ty: ColumnType::Text,
            },
        ],
        primary_key: 0,
    }
}

fn parse_id(value: &str) -> Result<i64> {
    value
        .parse::<i64>()
        .map_err(|_| DbError::InvalidInput(format!("invalid i64 primary key {value}")))
}

fn run() -> Result<()> {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 {
        return Err(usage());
    }
    let path = &args[1];
    let mut engine = RelationalEngine::open(path)?;
    match args[2].as_str() {
        "init" if args.len() == 3 => {
            let tx = engine.commit(&[RelOp::CreateTable {
                name: "users".to_owned(),
                schema: users_schema(),
            }])?;
            println!("tx={tx} table=users");
        }
        "put" if args.len() == 5 => {
            let id = parse_id(&args[3])?;
            let tx = engine.commit(&[RelOp::UpsertRow {
                table: "users".to_owned(),
                row: vec![Cell::Int64(id), Cell::Text(args[4].clone())],
            }])?;
            println!("tx={tx} upserted={id}");
        }
        "get" if args.len() == 4 => {
            let id = parse_id(&args[3])?;
            match engine.row("users", &Cell::Int64(id))? {
                Some(row) => println!("{}\t{}", display_cell(&row[0]), display_cell(&row[1])),
                None => println!("not-found"),
            }
        }
        "delete" if args.len() == 4 => {
            let id = parse_id(&args[3])?;
            let tx = engine.commit(&[RelOp::DeleteRow {
                table: "users".to_owned(),
                key: Cell::Int64(id),
            }])?;
            println!("tx={tx} deleted={id}");
        }
        "list" if args.len() == 3 => {
            for (_, row) in engine.rows("users")? {
                println!("{}\t{}", display_cell(&row[0]), display_cell(&row[1]));
            }
        }
        "catalog" if args.len() == 3 => {
            for (name, schema) in engine.catalog() {
                println!("{name}\tcolumns={}\tprimary_key={}", schema.columns.len(), schema.primary_key);
            }
        }
        _ => return Err(usage()),
    }
    Ok(())
}

fn usage() -> DbError {
    DbError::InvalidInput(
        "usage: db-log-relational <path> <init|put|get|delete|list|catalog> [args]".to_owned(),
    )
}

fn display_cell(cell: &Cell) -> String {
    match cell {
        Cell::Int64(value) => value.to_string(),
        Cell::Text(value) => value.clone(),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
