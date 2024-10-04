use duckdb::{Connection, Result};
use std::error::Error;
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
fn determine_file_type(file_content: &[u8]) -> io::Result<FileType> {
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
fn query_and_print_schema(conn: &Connection) -> Result<()> {
    let query = "SELECT * FROM data LIMIT 10";
    let mut stmt = conn.prepare(query)?;
    let arrow_result = stmt.query_arrow([])?;
    // Get the schema
    let schema = arrow_result.get_schema();
    println!("Schema: {:?}", schema);
    Ok(())
}

// Load to postgis
fn load_data_postgis(conn: &Connection, table_name: &str) -> Result<(), Box<dyn Error>> {
    // Attach PostGIS database
    conn.execute(
        "ATTACH 'dbname=gridwalk user=admin password=password host=localhost port=5432' AS gridwalk_db (TYPE POSTGRES)",
        [],
    )?;

    // Let table name
    let my_table_name = table_name;

    // Drop Table
    let delete_if_table_exists_query = &format!(
        "
        DROP TABLE IF EXISTS gridwalk_db.{};
    ",
        my_table_name
    );

    conn.execute(delete_if_table_exists_query, [])?;

    // Create Table
    let create_table_query = &format!(
        "
        CREATE TABLE gridwalk_db.{} AS
        SELECT *
        FROM data;
    ",
        my_table_name
    );

    conn.execute(create_table_query, [])?;

    // Postgis Update Table
    let postgis_query = &format!(
        "CALL postgres_execute('gridwalk_db', '
        ALTER TABLE {} ADD COLUMN geom geometry;
        UPDATE {} SET geom = ST_GeomFromText(geom_wkt, 4326);
        ALTER TABLE {} DROP COLUMN geom_wkt;
        ');",
        table_name, table_name, table_name
    );

    conn.execute(&postgis_query, [])?;

    println!(
        "Table {} created and data inserted successfully",
        my_table_name
    );
    Ok(())
}

// DuckDB file loader
fn process_file(file_path: &str, file_type: &FileType) -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute("INSTALL spatial;", [])?;
    conn.execute("LOAD spatial;", [])?;
    conn.execute("INSTALL postgres;", [])?;
    conn.execute("LOAD postgres;", [])?;

    let create_table_query = match file_type {
        FileType::Geopackage | FileType::Shapefile | FileType::Geojson => {
            format!(
                "CREATE TABLE data AS
                 SELECT * EXCLUDE (geom),
                 ST_AsText(geom) as geom_wkt
                 FROM ST_Read('{}');",
                file_path
            )
        }
        FileType::Excel => {
            format!(
                "CREATE TABLE data AS SELECT * FROM st_read('{}');",
                file_path
            )
        }
        FileType::Csv => {
            format!(
                "CREATE TABLE data AS SELECT * FROM read_csv('{}');",
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

    // Create the table in DuckDB
    conn.execute(&create_table_query, [])?;

    // Call to query and print data schema
    query_and_print_schema(&conn)?;

    // Call to load data into postgres and handle the result
    match load_data_postgis(&conn, "pop_tart") {
        Ok(_) => println!("Data successfully loaded into PostgreSQL"),
        Err(e) => eprintln!("Error loading data into PostgreSQL: {}", e),
    }

    Ok(())
}

// Process file
pub fn launch_process_file(file_path: &str) -> io::Result<()> {
    let mut file = File::open(file_path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    let file_type = determine_file_type(&buffer)?;
    println!("Detected file type: {:?}", file_type);

    process_file(file_path, &file_type).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("Error loading {:?} into DuckDB: {}", file_type, e),
        )
    })?;

    println!("Successfully loaded {:?} into DuckDB", file_type);
    Ok(())
}
