//! Immutable catalog-driven secondary index over a durable relational snapshot.
//!
//! The index borrows the relational engine immutably for its full lifetime. Rust therefore prevents
//! a caller from mutating that engine while the index is live, making the snapshot semantics
//! explicit instead of silently serving stale post-write results. Rebuilding after reopen derives
//! the same index from replayed durable state; no new on-disk format or persistence boundary is used.

use std::collections::{BTreeMap, BTreeSet};

use db_core::{DbError, Result};

use crate::query::{Projection, QueryResult};
use crate::relational::{Cell, ColumnType, RelationalEngine};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexKeyType {
    Int64,
    Text,
}

impl IndexKeyType {
    fn from_column_type(ty: &ColumnType) -> Self {
        match ty {
            ColumnType::Int64 => Self::Int64,
            ColumnType::Text => Self::Text,
        }
    }

    fn accepts(self, value: &Cell) -> bool {
        matches!(
            (self, value),
            (Self::Int64, Cell::Int64(_)) | (Self::Text, Cell::Text(_))
        )
    }
}

/// Read-only secondary index tied to one immutable relational-engine snapshot.
///
/// Entries map one typed column value to all matching rows. Rows within an equal-value bucket retain
/// the relational engine's primary-key order, including duplicate secondary-key matches.
pub struct SecondaryIndex<'a> {
    _engine: &'a RelationalEngine,
    table: String,
    column: String,
    key_type: IndexKeyType,
    columns: Vec<String>,
    entries: BTreeMap<Cell, Vec<Vec<Cell>>>,
}

impl std::fmt::Debug for SecondaryIndex<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecondaryIndex")
            .field("table", &self.table)
            .field("column", &self.column)
            .field("distinct_keys", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl<'a> SecondaryIndex<'a> {
    /// Builds a secondary index from the replayed durable state of `table`.
    pub fn build(engine: &'a RelationalEngine, table: &str, column: &str) -> Result<Self> {
        let schema = engine.schema(table)?;
        let column_index = schema
            .columns
            .iter()
            .position(|candidate| candidate.name == column)
            .ok_or_else(|| DbError::InvalidInput(format!("unknown column {column}")))?;
        let key_type = IndexKeyType::from_column_type(&schema.columns[column_index].ty);
        let columns = schema
            .columns
            .iter()
            .map(|candidate| candidate.name.clone())
            .collect();
        let mut entries: BTreeMap<Cell, Vec<Vec<Cell>>> = BTreeMap::new();
        for (_, row) in engine.rows(table)? {
            entries
                .entry(row[column_index].clone())
                .or_default()
                .push(row.to_vec());
        }
        Ok(Self {
            _engine: engine,
            table: table.to_owned(),
            column: column.to_owned(),
            key_type,
            columns,
            entries,
        })
    }

    /// Indexed table name.
    #[must_use]
    pub fn table(&self) -> &str {
        &self.table
    }

    /// Indexed column name.
    #[must_use]
    pub fn column(&self) -> &str {
        &self.column
    }

    /// Executes an equality lookup through this index with catalog-validated projection semantics.
    pub fn execute_eq(&self, value: &Cell, projection: &Projection) -> Result<QueryResult> {
        if !self.key_type.accepts(value) {
            return Err(DbError::InvalidInput(
                "index lookup literal type does not match indexed column type".to_owned(),
            ));
        }
        let projection = self.resolve_projection(projection)?;
        let columns = projection
            .iter()
            .map(|index| self.columns[*index].clone())
            .collect();
        let rows = self
            .entries
            .get(value)
            .into_iter()
            .flatten()
            .map(|row| projection.iter().map(|index| row[*index].clone()).collect())
            .collect();
        Ok(QueryResult { columns, rows })
    }

    fn resolve_projection(&self, projection: &Projection) -> Result<Vec<usize>> {
        match projection {
            Projection::All => Ok((0..self.columns.len()).collect()),
            Projection::Columns(columns) => {
                if columns.is_empty() {
                    return Err(DbError::InvalidInput(
                        "query projection must contain at least one column".to_owned(),
                    ));
                }
                let mut seen = BTreeSet::new();
                columns
                    .iter()
                    .map(|name| {
                        if !seen.insert(name.as_str()) {
                            return Err(DbError::InvalidInput(format!(
                                "duplicate projected column {name}"
                            )));
                        }
                        self.columns
                            .iter()
                            .position(|candidate| candidate == name)
                            .ok_or_else(|| DbError::InvalidInput(format!("unknown column {name}")))
                    })
                    .collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{self, CompareOp, Predicate, Query};
    use crate::relational::{Column, RelOp, Schema};
    use tempfile::tempdir;

    fn users_schema() -> Schema {
        Schema {
            columns: vec![
                Column {
                    name: "id".to_owned(),
                    ty: ColumnType::Int64,
                },
                Column {
                    name: "team".to_owned(),
                    ty: ColumnType::Text,
                },
                Column {
                    name: "name".to_owned(),
                    ty: ColumnType::Text,
                },
            ],
            primary_key: 0,
        }
    }

    fn seed(engine: &mut RelationalEngine) -> Result<()> {
        engine.commit(&[
            RelOp::CreateTable {
                name: "users".to_owned(),
                schema: users_schema(),
            },
            RelOp::UpsertRow {
                table: "users".to_owned(),
                row: vec![
                    Cell::Int64(3),
                    Cell::Text("systems".to_owned()),
                    Cell::Text("Edsger".to_owned()),
                ],
            },
            RelOp::UpsertRow {
                table: "users".to_owned(),
                row: vec![
                    Cell::Int64(1),
                    Cell::Text("languages".to_owned()),
                    Cell::Text("Ada".to_owned()),
                ],
            },
            RelOp::UpsertRow {
                table: "users".to_owned(),
                row: vec![
                    Cell::Int64(2),
                    Cell::Text("systems".to_owned()),
                    Cell::Text("Grace".to_owned()),
                ],
            },
        ])?;
        Ok(())
    }

    #[test]
    fn equality_index_matches_scan_and_preserves_primary_key_order() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("index.log");
        let mut engine = RelationalEngine::open(&path)?;
        seed(&mut engine)?;

        let index = SecondaryIndex::build(&engine, "users", "team")?;
        let indexed = index.execute_eq(
            &Cell::Text("systems".to_owned()),
            &Projection::Columns(vec!["id".to_owned(), "name".to_owned()]),
        )?;
        let scanned = query::execute(
            &engine,
            &Query {
                table: "users".to_owned(),
                predicate: Some(Predicate {
                    column: "team".to_owned(),
                    op: CompareOp::Eq,
                    value: Cell::Text("systems".to_owned()),
                }),
                projection: Projection::Columns(vec!["id".to_owned(), "name".to_owned()]),
            },
        )?;
        assert_eq!(indexed, scanned);
        assert_eq!(
            indexed.rows,
            vec![
                vec![Cell::Int64(2), Cell::Text("Grace".to_owned())],
                vec![Cell::Int64(3), Cell::Text("Edsger".to_owned())],
            ]
        );
        Ok(())
    }

    #[test]
    fn rebuilt_index_after_reopen_matches_scan_oracle() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("reopen-index.log");
        let mut engine = RelationalEngine::open(&path)?;
        seed(&mut engine)?;
        engine.commit(&[
            RelOp::DeleteRow {
                table: "users".to_owned(),
                key: Cell::Int64(3),
            },
            RelOp::UpsertRow {
                table: "users".to_owned(),
                row: vec![
                    Cell::Int64(4),
                    Cell::Text("systems".to_owned()),
                    Cell::Text("Barbara".to_owned()),
                ],
            },
        ])?;
        drop(engine);

        let engine = RelationalEngine::open(&path)?;
        let index = SecondaryIndex::build(&engine, "users", "team")?;
        let indexed = index.execute_eq(&Cell::Text("systems".to_owned()), &Projection::All)?;
        let scanned = query::execute(
            &engine,
            &Query {
                table: "users".to_owned(),
                predicate: Some(Predicate {
                    column: "team".to_owned(),
                    op: CompareOp::Eq,
                    value: Cell::Text("systems".to_owned()),
                }),
                projection: Projection::All,
            },
        )?;
        assert_eq!(indexed, scanned);
        assert_eq!(
            indexed
                .rows
                .iter()
                .map(|row| row[0].clone())
                .collect::<Vec<_>>(),
            vec![Cell::Int64(2), Cell::Int64(4)]
        );
        Ok(())
    }

    #[test]
    fn index_validation_rejects_unknown_columns_wrong_types_and_bad_projection() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("index-validation.log");
        let mut engine = RelationalEngine::open(&path)?;
        seed(&mut engine)?;

        let unknown = SecondaryIndex::build(&engine, "users", "missing")
            .expect_err("unknown indexed column must fail");
        assert!(matches!(unknown, DbError::InvalidInput(_)));

        let index = SecondaryIndex::build(&engine, "users", "team")?;
        let wrong_type = index
            .execute_eq(&Cell::Int64(7), &Projection::All)
            .expect_err("wrong lookup type must fail");
        assert!(matches!(wrong_type, DbError::InvalidInput(_)));

        let bad_projection = index
            .execute_eq(
                &Cell::Text("systems".to_owned()),
                &Projection::Columns(vec!["missing".to_owned()]),
            )
            .expect_err("unknown projection must fail");
        assert!(matches!(bad_projection, DbError::InvalidInput(_)));
        Ok(())
    }
}
