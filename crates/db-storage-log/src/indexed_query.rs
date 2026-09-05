//! Index-aware query execution over a durable relational snapshot.
//!
//! The executor owns one immutable secondary index and the engine snapshot it was built from. A
//! matching equality predicate uses the index; every other query falls back to the catalog-driven
//! scan executor. Both paths preserve the same validation, projection, and deterministic ordering
//! semantics, and neither changes the durable relational format or persistence boundary.

use db_core::Result;

use crate::index::SecondaryIndex;
use crate::query::{self, CompareOp, Query, QueryResult};
use crate::relational::RelationalEngine;

/// Query executor with one immutable secondary-index access path.
#[derive(Debug)]
pub struct IndexedQueryExecutor<'a> {
    engine: &'a RelationalEngine,
    index: SecondaryIndex<'a>,
}

impl<'a> IndexedQueryExecutor<'a> {
    /// Builds an index-aware executor for one catalog-validated table column.
    pub fn build(engine: &'a RelationalEngine, table: &str, column: &str) -> Result<Self> {
        Ok(Self {
            engine,
            index: SecondaryIndex::build(engine, table, column)?,
        })
    }

    /// Executes `query`, choosing the secondary index for a matching equality predicate.
    ///
    /// Queries that target another table, another predicate column, a non-equality predicate, or no
    /// predicate are delegated to the ordinary scan executor over the same immutable engine state.
    pub fn execute(&self, query: &Query) -> Result<QueryResult> {
        if query.table == self.index.table() {
            if let Some(predicate) = query.predicate.as_ref() {
                if predicate.column == self.index.column() && predicate.op == CompareOp::Eq {
                    return self.index.execute_eq(&predicate.value, &query.projection);
                }
            }
        }
        query::execute(self.engine, query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{Predicate, Projection};
    use crate::relational::{Cell, Column, ColumnType, RelOp, Schema};
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
    fn matching_equality_query_matches_scan_oracle() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("indexed-query.log");
        let mut engine = RelationalEngine::open(&path)?;
        seed(&mut engine)?;

        let query = Query {
            table: "users".to_owned(),
            predicate: Some(Predicate {
                column: "team".to_owned(),
                op: CompareOp::Eq,
                value: Cell::Text("systems".to_owned()),
            }),
            projection: Projection::Columns(vec!["id".to_owned(), "name".to_owned()]),
        };
        let scanned = query::execute(&engine, &query)?;
        let executor = IndexedQueryExecutor::build(&engine, "users", "team")?;
        let indexed = executor.execute(&query)?;

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
    fn nonmatching_predicates_fall_back_to_scan_semantics() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("indexed-fallback.log");
        let mut engine = RelationalEngine::open(&path)?;
        seed(&mut engine)?;
        let executor = IndexedQueryExecutor::build(&engine, "users", "team")?;

        for query in [
            Query {
                table: "users".to_owned(),
                predicate: Some(Predicate {
                    column: "team".to_owned(),
                    op: CompareOp::Ge,
                    value: Cell::Text("systems".to_owned()),
                }),
                projection: Projection::All,
            },
            Query {
                table: "users".to_owned(),
                predicate: Some(Predicate {
                    column: "id".to_owned(),
                    op: CompareOp::Gt,
                    value: Cell::Int64(1),
                }),
                projection: Projection::Columns(vec!["name".to_owned()]),
            },
        ] {
            assert_eq!(executor.execute(&query)?, query::execute(&engine, &query)?);
        }
        Ok(())
    }

    #[test]
    fn rebuilt_executor_after_reopen_matches_scan_oracle() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("indexed-reopen.log");
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
        let query = Query {
            table: "users".to_owned(),
            predicate: Some(Predicate {
                column: "team".to_owned(),
                op: CompareOp::Eq,
                value: Cell::Text("systems".to_owned()),
            }),
            projection: Projection::All,
        };
        let executor = IndexedQueryExecutor::build(&engine, "users", "team")?;
        assert_eq!(executor.execute(&query)?, query::execute(&engine, &query)?);
        Ok(())
    }
}
