use arrow_h3::h3ron::{H3Cell, Index};
use arrow_h3::polars::frame::DataFrame;
use arrow_h3::polars::prelude::NamedFrom;
use arrow_h3::polars::series::Series;
use arrow_h3::H3DataFrame;
use chrono::Local;
use h3cellstore::clickhouse::clickhouse_arrow_grpc::{ArrowInterface, ClickHouseClient, QueryInfo};
use h3cellstore::clickhouse::compacted_tables::schema::{
    AggregationMethod, ClickhouseDataType, ColumnDefinition, CompactedTableSchema,
    CompactedTableSchemaBuilder, SimpleColumn, TemporalPartitioning,
};
use h3cellstore::clickhouse::compacted_tables::CompactedTablesStore;
use h3cellstore::clickhouse::COL_NAME_H3INDEX;

const MAX_H3_RES: u8 = 5;

fn okavango_delta_schema() -> eyre::Result<CompactedTableSchema> {
    let schema = CompactedTableSchemaBuilder::new("okavango_delta")
        .h3_base_resolutions((0..=MAX_H3_RES).collect())
        .temporal_partitioning(TemporalPartitioning::Month)
        .add_column(
            "elephant_count",
            ColumnDefinition::WithAggregation(
                SimpleColumn::new(ClickhouseDataType::UInt32, None),
                AggregationMethod::Sum,
            ),
        )
        .add_column(
            "observed_on",
            ColumnDefinition::Simple(SimpleColumn::new(ClickhouseDataType::DateTime64, Some(0))),
        )
        .build()?;
    Ok(schema)
}

fn make_h3dataframe() -> eyre::Result<H3DataFrame> {
    let h3indexes = H3Cell::from_coordinate((22.8996, -19.3325).into(), MAX_H3_RES)?
        .grid_disk(10)?
        .iter()
        .map(|cell| cell.h3index() as u64)
        .collect::<Vec<_>>();

    let num_cells = h3indexes.len();
    let df = DataFrame::new(vec![
        Series::new(COL_NAME_H3INDEX, h3indexes),
        Series::new(
            "elephant_count",
            (0..num_cells).map(|_| 2_u32).collect::<Vec<_>>(),
        ),
        Series::new(
            "observed_on",
            (0..num_cells)
                .map(|_| Local::now().naive_local())
                .collect::<Vec<_>>(),
        ),
    ])?;

    Ok(H3DataFrame::from_dataframe(df, COL_NAME_H3INDEX)?)
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    let mut client = ClickHouseClient::connect("http://127.0.0.1:9100")
        .await?
        .send_gzip()
        .accept_gzip();

    let play_db = "play";
    client
        .execute_query_checked(QueryInfo {
            query: format!("create database if not exists {}", play_db),
            ..Default::default()
        })
        .await?;

    let schema = okavango_delta_schema()?;
    client.create_tableset_schema(&play_db, &schema).await?;

    let tablesets = client.list_tablesets(&play_db).await?;
    assert!(tablesets.contains_key("okavango_delta"));

    let h3df = make_h3dataframe()?;

    client
        .insert_h3dataframe_into_tableset(&play_db, &schema, h3df, false)
        .await?;

    /*
    client.drop_tableset(&play_db, "okavango_delta").await?;
    assert!(!client
        .list_tablesets(&play_db)
        .await?
        .contains_key("okavango_delta"));

     */

    Ok(())
}
