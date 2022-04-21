use arrow_h3::h3ron::collections::{HashMap, HashSet};
use lazy_static::lazy_static;

use arrow_h3::h3ron::{H3Cell, Index, H3_MIN_RESOLUTION};
use itertools::Itertools;
use regex::Regex;

use super::COL_NAME_H3INDEX;
use crate::Error;

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct TableSpec {
    pub h3_resolution: u8,
    pub is_compacted: bool,

    /// temporary tables are just used during ingestion of new data
    /// into the clickhouse db
    pub temporary_key: Option<String>,

    /// describes if the tables use the _base suffix
    pub has_base_suffix: bool,
}

impl TableSpec {
    pub fn is_temporary(&self) -> bool {
        self.temporary_key.is_some()
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct Table {
    pub basename: String,
    pub spec: TableSpec,
}

lazy_static! {
    static ref RE_TABLE: Regex = Regex::new(
        r"^([a-zA-Z].[a-zA-Z_0-9]+)_([0-9]{2})(_(base|compacted))?(_tmp([a-zA-Z0-9_]+))?$"
    )
    .unwrap();
}

impl Table {
    pub fn parse(full_table_name: &str) -> Option<Table> {
        RE_TABLE.captures(full_table_name).map(|captures| Table {
            basename: captures[1].to_string(),
            spec: TableSpec {
                h3_resolution: captures[2].parse().unwrap(),
                is_compacted: if let Some(suffix) = captures.get(4) {
                    suffix.as_str() == "compacted"
                } else {
                    false
                },
                temporary_key: captures.get(6).map(|mtch| mtch.as_str().to_string()),
                has_base_suffix: captures.get(4).is_some(),
            },
        })
    }

    pub fn to_table_name(&self) -> String {
        format!(
            "{}_{:02}{}{}",
            self.basename,
            self.spec.h3_resolution,
            // the suffix
            if self.spec.has_base_suffix {
                if self.spec.is_compacted {
                    "_compacted"
                } else {
                    "_base"
                }
            } else {
                ""
            },
            // the temporary key
            if let Some(temp_key) = &self.spec.temporary_key {
                format!("_tmp{}", temp_key)
            } else {
                "".to_string()
            }
        )
    }
}

impl ToString for Table {
    fn to_string(&self) -> String {
        self.to_table_name()
    }
}

#[derive(Clone)]
pub enum TableSetQuery {
    /// autogenerate a query based on the available columns
    AutoGenerated,

    /// templated select statement
    ///
    /// The selected columns must include the h3indexes in a column named `h3index`
    ///
    /// The query must include these placeholders:
    /// * "<[table]>": will be filled with the table to be queried
    /// * "<[h3indexes]>": will be filled with an array of h3indexes used for the query
    ///
    /// TODO: parsing and validating and injecting missing column into the query with https://github.com/ballista-compute/sqlparser-rs
    ///    would be nice, but as the parser does not implement a clickhouse dialect, its is probably more
    ///    error prone than it is beneficial.
    TemplatedSelect(String),
}

impl TableSetQuery {
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            TableSetQuery::AutoGenerated => Ok(()),
            TableSetQuery::TemplatedSelect(querystring) => {
                for placeholder in &["<[table]>", "<[h3indexes]>"] {
                    if !querystring.contains(placeholder) {
                        return Err(Error::MissingQueryPlaceholder(placeholder.to_string()));
                    }
                }
                Ok(())
            }
        }
    }
}

impl From<Option<String>> for TableSetQuery {
    fn from(instr: Option<String>) -> Self {
        match instr {
            Some(s) => Self::TemplatedSelect(s),
            None => Self::AutoGenerated,
        }
    }
}

#[derive(Clone)]
pub struct TableSet {
    pub basename: String,
    pub columns: HashMap<String, String>,
    pub base_tables: HashMap<u8, TableSpec>,
    pub compacted_tables: HashMap<u8, TableSpec>,
}

impl TableSet {
    fn new(basename: &str) -> TableSet {
        TableSet {
            basename: basename.to_string(),
            compacted_tables: Default::default(),
            base_tables: Default::default(),
            columns: Default::default(),
        }
    }

    pub fn base_resolutions(&self) -> Vec<u8> {
        self.base_tables.keys().sorted_unstable().cloned().collect()
    }

    pub fn compacted_resolutions(&self) -> Vec<u8> {
        self.compacted_tables
            .keys()
            .sorted_unstable()
            .cloned()
            .collect()
    }

    pub fn compacted_tables(&self) -> Vec<Table> {
        let mut tables = Vec::new();
        for (_, table_spec) in self.compacted_tables.iter() {
            let t = Table {
                basename: self.basename.clone(),
                spec: table_spec.clone(),
            };
            tables.push(t);
        }
        tables
    }

    pub fn base_tables(&self) -> Vec<Table> {
        let mut tables = Vec::new();
        for (_, table_spec) in self.base_tables.iter() {
            let t = Table {
                basename: self.basename.clone(),
                spec: table_spec.clone(),
            };
            tables.push(t);
        }
        tables
    }

    pub fn tables(&self) -> Vec<Table> {
        let mut tables = self.base_tables();
        tables.append(&mut self.compacted_tables());
        tables
    }

    pub fn num_tables(&self) -> usize {
        self.base_tables.len() + self.compacted_tables.len()
    }

    /// build a select query for the given h3 cells.
    ///
    /// Will also fetch the parent, compacted indexes.
    pub(crate) fn build_select_query(
        &self,
        cells: &[H3Cell],
        query: &TableSetQuery,
    ) -> Result<String, Error> {
        query.validate()?;

        // use the h3 resolution of the first index as the target resolution
        let h3_resolution = if let Some(cell) = cells.first() {
            cell.resolution()
        } else {
            return Err(Error::EmptyCells);
        };

        // collect the indexes and the parents (where the tables exist)
        let mut queryable_h3indexes: HashMap<_, HashSet<_>> = self
            .base_tables
            .iter()
            .chain(self.compacted_tables.iter())
            .filter(|(r, _)| **r <= h3_resolution)
            .map(|(r, _)| (*r, HashSet::new()))
            .collect();
        for cell in cells {
            if cell.resolution() != h3_resolution {
                return Err(Error::MixedH3Resolutions);
            }
            for (resolution, queryable_h3indexes_set) in queryable_h3indexes.iter_mut() {
                queryable_h3indexes_set.insert(cell.get_parent(*resolution)?.h3index());
            }
        }
        if queryable_h3indexes.is_empty() {
            return Err(Error::NoQueryableTables);
        }

        let query_string = {
            let selectable_columns = itertools::join(
                self.columns
                    .iter()
                    .map(|(col_name, _)| col_name)
                    .filter(|col_name| !col_name.starts_with(COL_NAME_H3INDEX)),
                ", ",
            );

            let mut query_string_parts = Vec::new();
            for r in H3_MIN_RESOLUTION..=h3_resolution {
                if let Some(query_h3indexes) = queryable_h3indexes.get(&r) {
                    let query_h3indexesarray_string = format!(
                        "[{}]",
                        itertools::join(query_h3indexes.iter().map(|hi| hi.to_string()), ",",)
                    );

                    let tablename = Table {
                        basename: self.basename.clone(),
                        spec: TableSpec {
                            h3_resolution: r,
                            is_compacted: r != h3_resolution,
                            temporary_key: None,
                            has_base_suffix: if r != h3_resolution {
                                &self.compacted_tables
                            } else {
                                &self.base_tables
                            }
                            .get(&r)
                            .map_or_else(|| true, |table_spec| table_spec.has_base_suffix),
                        },
                    }
                    .to_table_name();

                    query_string_parts.push(match &query {
                        TableSetQuery::AutoGenerated => {
                            format!(
                                "select {}, {} from {} where {} in {}",
                                COL_NAME_H3INDEX,
                                selectable_columns,
                                tablename,
                                COL_NAME_H3INDEX,
                                query_h3indexesarray_string
                            )
                        }
                        TableSetQuery::TemplatedSelect(query_string) => query_string
                            .replace("<[table]>", &tablename)
                            .replace("<[h3indexes]>", &query_h3indexesarray_string),
                    });
                }
            }

            itertools::join(query_string_parts.iter(), " union all ")
        };
        Ok(query_string)
    }
}

impl AsRef<str> for TableSet {
    fn as_ref(&self) -> &str {
        &self.basename
    }
}

/// identify the tablesets from a slice of tablenames
pub(crate) fn find_tablesets<T: AsRef<str>>(tablenames: &[T]) -> HashMap<String, TableSet> {
    let mut tablesets = HashMap::default();

    for tablename in tablenames.iter() {
        if let Some(table) = Table::parse(tablename.as_ref()) {
            if table.spec.is_temporary() {
                // ignore temporary tables here for now
                continue;
            }

            let tableset = tablesets
                .entry(table.basename.to_string())
                .or_insert_with(|| TableSet::new(&table.basename));
            if table.spec.is_compacted {
                tableset
                    .compacted_tables
                    .insert(table.spec.h3_resolution, table.spec);
            } else {
                tableset
                    .base_tables
                    .insert(table.spec.h3_resolution, table.spec);
            }
        }
    }
    tablesets
}

#[cfg(test)]
mod tests {
    use crate::clickhouse::compacted_tables::temporary_key::TemporaryKey;
    use crate::clickhouse::tableset::{find_tablesets, Table, TableSpec};

    #[test]
    fn test_table_to_name() {
        let mut table = Table {
            basename: "some_table".to_string(),
            spec: TableSpec {
                h3_resolution: 5,
                is_compacted: false,
                temporary_key: None,
                has_base_suffix: true,
            },
        };

        assert_eq!(table.to_table_name(), "some_table_05_base");

        table.spec.has_base_suffix = false;
        assert_eq!(table.to_table_name(), "some_table_05");
    }

    #[test]
    fn test_table_from_name_with_suffix() {
        let table = Table::parse("some_ta78ble_05_base");
        assert!(table.is_some());
        let table_u = table.unwrap();
        assert_eq!(table_u.basename, "some_ta78ble".to_string());
        assert_eq!(table_u.spec.h3_resolution, 5_u8);
        assert!(!table_u.spec.is_compacted);
        assert!(!table_u.spec.is_temporary());
    }

    #[test]
    fn test_table_from_name_without_suffix() {
        let table = Table::parse("some_ta78ble_05");
        assert!(table.is_some());
        let table_u = table.unwrap();
        assert_eq!(table_u.basename, "some_ta78ble".to_string());
        assert_eq!(table_u.spec.h3_resolution, 5_u8);
        assert!(!table_u.spec.is_compacted);
        assert!(!table_u.spec.is_temporary());
    }

    #[test]
    fn test_table_from_name_temporary_temporarykey() {
        let temporary_key = TemporaryKey::new();
        let table = Table {
            basename: "some_table".to_string(),
            spec: TableSpec {
                h3_resolution: 5,
                is_compacted: false,
                temporary_key: Some(temporary_key.to_string()),
                has_base_suffix: true,
            },
        };
        let table2 = Table::parse(&table.to_table_name()).unwrap();
        assert_eq!(table, table2);
        assert_eq!(
            temporary_key.to_string(),
            table2.spec.temporary_key.unwrap()
        );
    }

    #[test]
    fn test_table_from_name_temporary_without_suffix() {
        let table = Table::parse("some_ta78ble_05_tmp5t");
        assert!(table.is_some());
        let table_u = table.unwrap();
        assert_eq!(table_u.basename, "some_ta78ble".to_string());
        assert_eq!(table_u.spec.h3_resolution, 5_u8);
        assert!(!table_u.spec.is_compacted);
        assert!(table_u.spec.is_temporary());
        assert_eq!(table_u.spec.temporary_key, Some("5t".to_string()));
    }

    #[test]
    fn test_table_from_name_temporary_with_suffix() {
        let table = Table::parse("some_ta78ble_05_base_tmp5t");
        assert!(table.is_some());
        let table_u = table.unwrap();
        assert_eq!(table_u.basename, "some_ta78ble".to_string());
        assert_eq!(table_u.spec.h3_resolution, 5_u8);
        assert!(!table_u.spec.is_compacted);
        assert!(table_u.spec.is_temporary());
        assert_eq!(table_u.spec.temporary_key, Some("5t".to_string()));
    }

    #[test]
    fn test_find_tablesets() {
        let table_names = [
            "aggregate_function_combinators",
            "asynchronous_metrics",
            "build_options",
            "clusters",
            "collations",
            "columns",
            "contributors",
            "something_else_06_base",
            "something_else_07_base",
            "data_type_families",
            "databases",
            "detached_parts",
            "dictionaries",
            "disks",
            "events",
            "formats",
            "functions",
            "graphite_retentions",
            "macros",
            "merge_tree_settings",
            "merges",
            "metric_log",
            "metrics",
            "models",
            "mutations",
            "numbers",
            "numbers_mt",
            "one",
            "parts",
            "parts_columns",
            "processes",
            "quota_usage",
            "quotas",
            "replicas",
            "replication_queue",
            "row_policies",
            "settings",
            "stack_trace",
            "storage_policies",
            "table_engines",
            "table_functions",
            "tables",
            "trace_log",
            "zeros",
            "zeros_mt",
            "water_00_base",
            "water_00_compacted",
            "water_01_base",
            "water_01_compacted",
            "water_02_base",
            "water_02_compacted",
            "water_03_base",
            "water_03_compacted",
            "water_04_base",
            "water_04_compacted",
            "water_05_base",
            "water_05_compacted",
            "water_06_base",
            "water_06_compacted",
            "water_07_base",
            "water_07_compacted",
            "water_08_base",
            "water_08_compacted",
            "water_09_base",
            "water_09_compacted",
            "water_10_base",
            "water_10_compacted",
            "water_11_base",
            "water_11_compacted",
            "water_12_base",
            "water_12_compacted",
            "water_13_base",
            "water_13_compacted",
            "elephants_02",
            "elephants_03",
            "elephants_01_compacted",
        ];

        let tablesets = find_tablesets(&table_names);
        assert_eq!(tablesets.len(), 3);
        assert!(tablesets.contains_key("water"));
        let water_ts = tablesets.get("water").unwrap();
        assert_eq!(water_ts.basename, "water");
        for h3res in 0..=13 {
            assert!(water_ts.base_tables.get(&h3res).is_some());
            assert!(water_ts.compacted_tables.get(&h3res).is_some());
        }
        assert!(water_ts.base_tables.get(&14).is_none());
        assert!(water_ts.compacted_tables.get(&14).is_none());

        assert!(tablesets.contains_key("something_else"));
        let se_ts = tablesets.get("something_else").unwrap();
        assert_eq!(se_ts.basename, "something_else");
        assert_eq!(se_ts.base_tables.len(), 2);
        assert!(se_ts.base_tables.get(&6).is_some());
        assert!(se_ts.base_tables.get(&7).is_some());
        assert_eq!(se_ts.compacted_tables.len(), 0);

        assert!(tablesets.contains_key("elephants"));
        let elephants_ts = tablesets.get("elephants").unwrap();
        assert_eq!(elephants_ts.basename, "elephants");
        assert_eq!(elephants_ts.base_tables.len(), 2);
        assert!(elephants_ts.base_tables.get(&2).is_some());
        assert!(elephants_ts.base_tables.get(&3).is_some());
        assert_eq!(elephants_ts.compacted_tables.len(), 1);
        assert!(elephants_ts.compacted_tables.get(&1).is_some());
    }
}
