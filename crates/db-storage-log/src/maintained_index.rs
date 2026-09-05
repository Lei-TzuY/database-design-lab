//! Transaction-maintained secondary indexes over the durable relational engine.
//!
//! Index definitions are process-local in this bounded Phase 5 slice. Durable relational commits
//! still use the unchanged v1 transaction record and append/sync boundary. After each successful
//! commit, materialized indexes are rebuilt from the exact committed engine state before subsequent
//! indexed queries are served. Failed relational commits leave both durable state and indexes
//! unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use db_core::{DbError, Result};

use crate::query::{self, CompareOp, Projection, Query, QueryResult};
use crate::relational::{Cell, ColumnType, RelOp, RelationalEngine};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct IndexSpec {
    table: String,
    column: String,
}

#[derive(Debug, Clone)]
struct MaterializedIndex {
    spec: IndexSpec,
    key_type: ColumnType,
    columns: Vec<String>,
    entries: BTreeMap<Cell, Vec<Vec<Cell>>>,
}

impl MaterializedIndex {
    fn build(engine: &RelationalEngine, spec: IndexSpec) -> Result<Self> {
        let schema = engine.schema(&spec.table)?;
        let column_index = schema
            .columns
            .iter()
            .position(|candidate| candidate.name == spec.column)
            .ok_or_else(|| DbError::InvalidInput(format!("unknown column {}", spec.column)))?;
        let key_type = schema.columns[column_index].ty.clone();
        let columns = schema
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
        let mut entries: BTreeMap<Cell, Vec<Vec<Cell>>> = BTreeMap::new();
        for (_, row) in engine.rows(&spec.table)? {
            entries
                .entry(row[column_index].clone())
                .or_default()
                .push(row.to_vec());
        }
        Ok(Self {
            spec,
            key_type,
            columns,
            entries,
        })
    }

    fn execute_eq(&self, value: &Cell, projection: &Projection) -> Result<QueryResult> {
        let type_matches = matches!(
            (&self.key_type, value),
            (ColumnType::Int64, Cell::Int64(_)) | (ColumnType::Text, Cell::Text(_))
        );
        if !type_matches {
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

/// Durable relational engine with process-local secondary indexes maintained across commits.
#[derive(Debug)]
pub struct MaintainedIndexEngine {
    engine: RelationalEngine,
    indexes: BTreeMap<IndexSpec, MaterializedIndex>,
}

impl MaintainedIndexEngine {
    /// Opens or creates a durable relational database with no registered process-local indexes.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            engine: RelationalEngine::open(path)?,
            indexes: BTreeMap::new(),
        })
    }

    /// Registers and materializes one catalog-validated secondary index.
    pub fn register_index(&mut self, table: &str, column: &str) -> Result<()> {
        let spec = IndexSpec {
            table: table.to_owned(),
            column: column.to_owned(),
        };
        if self.indexes.contains_key(&spec) {
            return Err(DbError::InvalidInput(format!(
                "secondary index already registered for {table}.{column}"
            )));
        }
        let index = MaterializedIndex::build(&self.engine, spec.clone())?;
        self.indexes.insert(spec, index);
        Ok(())
    }

    /// Atomically commits relational operations through the existing durable boundary and refreshes
    /// all registered indexes from the committed state before returning success.
    pub fn commit(&mut self, ops: &[RelOp]) -> Result<u64> {
        let tx_id = self.engine.commit(ops)?;
        self.rebuild_indexes()?;
        Ok(tx_id)
    }

    /// Executes a query through a matching registered equality index when available, otherwise
    /// delegates to the catalog-driven scan executor.
    pub fn execute(&self, query: &Query) -> Result<QueryResult> {
        if let Some(predicate) = query.predicate.as_ref() {
            if predicate.op == CompareOp::Eq {
                let spec = IndexSpec {
                    table: query.table.clone(),
                    column: predicate.column.clone(),
                };
                if let Some(index) = self.indexes.get(&spec) {
                    return index.execute_eq(&predicate.value, &query.projection);
                }
            }
        }
        query::execute(&self.engine, query)
    }

    /// Access to the underlying durable relational state for catalog/read-only inspection.
    #[must_use]
    pub fn relational(&self) -> &RelationalEngine {
        &self.engine
    }

    fn rebuild_indexes(&mut self) -> Result<()> {
        let specs = self.indexes.keys().cloned().collect::<Vec<_>>();
        let mut rebuilt = BTreeMap::new();
        for spec in specs {
            rebuilt.insert(spec.clone(), MaterializedIndex::build(&self.engine, spec)?);
        }
        self.indexes = rebuilt;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{Predicate, Projection};
    use crate::relational::{Column, Schema};
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

    fn query_team(team: &str) -> Query {
        Query {
            table: "users".to_owned(),
            predicate: Some(Predicate {
                column: "team".to_owned(),
                op: CompareOp::Eq,
                value: Cell::Text(team.to_owned()),
            }),
            projection: Projection::All,
        }
    }

    fn seed(engine: &mut MaintainedIndexEngine) -> Result<()> {
        engine.commit(&[
            RelOp::CreateTable {
                name: "users".to_owned(),
                schema: users_schema(),
            },
            RelOp::UpsertRow {
                table: "users".to_owned(),
                row: vec![
                    Cell::Int64(1),
                    Cell::Text("systems".to_owned()),
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
        engine.register_index("users", "team")?;
        Ok(())
    }

    #[test]
    fn maintained_index_tracks_upsert_bucket_moves_and_deletes() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("maintained.log");
        let mut engine = MaintainedIndexEngine::open(&path)?;
        seed(&mut engine)?;

        engine.commit(&[
            RelOp::UpsertRow {
                table: "users".to_owned(),
                row: vec![
                    Cell::Int64(2),
                    Cell::Text("languages".to_owned()),
                    Cell::Text("Grace".to_owned()),
                ],
            },
            RelOp::UpsertRow {
                table: "users".to_owned(),
                row: vec![
                    Cell::Int64(3),
                    Cell::Text("systems".to_owned()),
                    Cell::Text("Edsger".to_owned()),
                ],
            },
            RelOp::DeleteRow {
                table: "users".to_owned(),
                key: Cell::Int64(1),
            },
        ])?;

        for query in [query_team("systems"), query_team("languages")] {
            assert_eq!(
                engine.execute(&query)?,
                query::execute(engine.relational(), &query)?
            );
        }
        assert_eq!(
            engine.execute(&query_team("systems"))?.rows,
            vec![vec![
                Cell::Int64(3),
                Cell::Text("systems".to_owned()),
                Cell::Text("Edsger".to_owned()),
            ]]
        );
        Ok(())
    }

    #[test]
    fn duplicate_secondary_keys_stay_in_primary_key_order() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("duplicates.log");
        let mut engine = MaintainedIndexEngine::open(&path)?;
        seed(&mut engine)?;
        engine.commit(&[RelOp::UpsertRow {
            table: "users".to_owned(),
            row: vec![
                Cell::Int64(0),
                Cell::Text("systems".to_owned()),
                Cell::Text("Barbara".to_owned()),
            ],
        }])?;

        let rows = engine.execute(&query_team("systems"))?.rows;
        assert_eq!(
            rows.iter().map(|row| row[0].clone()).collect::<Vec<_>>(),
            vec![Cell::Int64(0), Cell::Int64(1), Cell::Int64(2)]
        );
        Ok(())
    }

    #[test]
    fn failed_commit_leaves_index_and_durable_state_unchanged() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("failed.log");
        let mut engine = MaintainedIndexEngine::open(&path)?;
        seed(&mut engine)?;
        let before = engine.execute(&query_team("systems"))?;
        let next_tx = engine.relational().next_transaction_id();

        let error = engine
            .commit(&[RelOp::UpsertRow {
                table: "missing".to_owned(),
                row: vec![Cell::Int64(9)],
            }])
            .expect_err("invalid transaction must fail");
        assert!(matches!(error, DbError::InvalidInput(_)));
        assert_eq!(engine.relational().next_transaction_id(), next_tx);
        assert_eq!(engine.execute(&query_team("systems"))?, before);
        Ok(())
    }

    #[test]
    fn reopen_and_reregister_rebuilds_equivalent_indexes() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("reopen.log");
        let mut engine = MaintainedIndexEngine::open(&path)?;
        seed(&mut engine)?;
        engine.commit(&[RelOp::UpsertRow {
            table: "users".to_owned(),
            row: vec![
                Cell::Int64(4),
                Cell::Text("systems".to_owned()),
                Cell::Text("Barbara".to_owned()),
            ],
        }])?;
        drop(engine);

        let mut reopened = MaintainedIndexEngine::open(&path)?;
        reopened.register_index("users", "team")?;
        let query = query_team("systems");
        assert_eq!(
            reopened.execute(&query)?,
            query::execute(reopened.relational(), &query)?
        );
        Ok(())
    }
}
