use duckdb::arrow::record_batch::RecordBatch;
use duckdb::arrow::util::pretty::print_batches;
use duckdb::{Connection, Result};
use std::fs::File;
use std::io::{self, Read};

// Enum to hold file types to match on
#[derive(Debug, PartialEq)]
pub enum FileType {
    Geopackage,
    Shapefile,
    Geojson,
    Excel,
    Csv,
    Parquet,
}

// Determine the file type that is being processed
pub fn determine_file_type(file_content: &[u8]) -> io::Result<FileType> {
    let header = &file_content[0..16.min(file_content.len())];
    if &header[0..4] == b"PK\x03\x04" {
        Ok(FileType::Excel)
    } else if &header[0..16] == b"SQLite format 3\0" {
        Ok(FileType::Geopackage)
    } else if &header[0..4] == b"\x00\x00\x27\x0A" {
        Ok(FileType::Shapefile)
    } else if &header[0..4] == b"PAR1" {
        Ok(FileType::Parquet)
    } else if header.starts_with(b"{") {
        let json_start = std::str::from_utf8(file_content).unwrap_or("");
        if json_start.contains("\"type\":")
            && (json_start.contains("\"FeatureCollection\"") || json_start.contains("\"Feature\""))
        {
            Ok(FileType::Geojson)
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a valid GeoJSON file",
            ))
        }
    } else {
        let file_text = std::str::from_utf8(file_content).unwrap_or("");
        let lines: Vec<&str> = file_text.lines().collect();
        if lines.len() >= 2
            && lines[0].split(',').count() > 1
            && lines[1].split(',').count() == lines[0].split(',').count()
            && file_text.is_ascii()
        {
            Ok(FileType::Csv)
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unknown file type",
            ))
        }
    }
}

// Get data schema
fn query_and_print_schema(conn: &Connection, query: &str, limit: usize) -> Result<()> {
    let mut stmt = conn.prepare(&format!("{} LIMIT {}", query, limit))?;
    let arrow_result = stmt.query_arrow([])?;

    // Get the schema
    let schema = arrow_result.get_schema();
    println!("Schema: {:?}", schema);

    // Collect RecordBatches
    let rbs: Vec<RecordBatch> = arrow_result.collect();

    // Calculate total number of rows
    let total_rows: usize = rbs.iter().map(|rb| rb.num_rows()).sum();

    // Print batches
    match print_batches(&rbs) {
        Ok(_) => println!("Successfully printed {} rows of data.", total_rows),
        Err(e) => eprintln!("Error printing batches: {}", e),
    }

    println!("Total number of rows in the result: {}", total_rows);

    Ok(())
}

// DuckDB file loader
pub fn load_file_duckdb(file_path: &str, file_type: &FileType) -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute("INSTALL spatial;", [])?;
    conn.execute("LOAD spatial;", [])?;

    let create_table_query = match file_type {
        FileType::Geopackage | FileType::Shapefile | FileType::Geojson => {
            format!(
                "CREATE TABLE data AS SELECT * FROM ST_Read('{}');",
                file_path
            )
        }
        FileType::Excel => {
            format!(
                "CREATE TABLE data AS SELECT * FROM read_excel('{}');",
                file_path
            )
        }
        FileType::Csv => {
            format!(
                "CREATE TABLE data AS SELECT * FROM read_csv_auto('{}');",
                file_path
            )
        }
        FileType::Parquet => {
            format!(
                "CREATE TABLE data AS SELECT * FROM parquet_scan('{}');",
                file_path
            )
        }
    };

    conn.execute(&create_table_query, [])?;

    // Call the private function to query and print record batches
    query_and_print_schema(&conn, "SELECT * FROM data", 5)?;

    Ok(())
}

// Process file
pub fn process_file(file_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::open(file_path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    match determine_file_type(&buffer) {
        Ok(file_type) => {
            println!("Detected file type: {:?}", file_type);
            match load_file_duckdb(file_path, &file_type) {
                Ok(_) => println!("Successfully loaded {:?} into DuckDB", file_type),
                Err(e) => println!("Error loading {:?} into DuckDB: {}", file_type, e),
            }
        }
        Err(e) => println!("Error determining file type: {}", e),
    }
    Ok(())
}
