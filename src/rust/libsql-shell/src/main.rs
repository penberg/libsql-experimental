use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use clap::Parser;
use rusqlite::{types::ValueRef, Connection, Statement};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

#[derive(Debug, Parser)]
#[command(name = "libsql")]
#[command(about = "libSQL client", long_about = None)]
struct Args {
    #[clap()]
    db_path: Option<String>,
}

// Presents libSQL values in human-readable form
fn format_value(v: ValueRef) -> String {
    match v {
        ValueRef::Null => "null".to_owned(),
        ValueRef::Integer(i) => format!("{i}"),
        ValueRef::Real(r) => format!("{r}"),
        ValueRef::Text(s) => std::str::from_utf8(s).unwrap().to_owned(),
        ValueRef::Blob(b) => format!("0x{}", general_purpose::STANDARD_NO_PAD.encode(b)),
    }
}

// Executes a libSQL statement
// TODO: introduce paging for presenting large results, get rid of Vec
fn execute(stmt: &mut Statement) -> Result<Vec<Vec<String>>> {
    let column_count = stmt.column_count();

    let rows = stmt.query_map((), |row| {
        let row = (0..column_count)
            .map(|idx| format_value(row.get_ref(idx).unwrap()))
            .collect::<Vec<String>>();
        Ok(row)
    })?;
    Ok(rows.map(|r| r.unwrap()).collect())
}

struct StrStatements {
    value: String,
}

impl Iterator for StrStatements {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        let mut embedded = false;
        let mut pos = 0;
        for (index, char) in self.value.chars().enumerate() {
            if char == '\'' {
                embedded = !embedded;
                continue;
            }
            if embedded || char != ';' {
                continue;
            }
            let str_statement = self.value[pos..index + 1].to_string();
            if str_statement.starts_with(';') || str_statement.is_empty() {
                pos = index + 1;
                continue;
            }
            self.value = self.value[index + 1..].to_string();
            return Some(str_statement.trim().to_string());
        }
        None
    }
}

fn get_str_statements(str: String) -> StrStatements {
    StrStatements { value: str }
}

fn run_statement(connection: &Connection, statement: String) {
    for str_statement in get_str_statements(statement) {
        let mut stmt = match connection.prepare(&str_statement) {
            Ok(stmt) => stmt,
            Err(e) => {
                println!("Error: {e}");
                continue;
            }
        };
        let rows = match execute(&mut stmt) {
            Ok(rows) => rows,
            Err(e) => {
                println!("Error: {e}");
                continue;
            }
        };
        if rows.is_empty() {
            continue;
        }
        let mut builder = tabled::builder::Builder::new();
        builder.set_columns(stmt.column_names());
        for row in rows {
            builder.add_record(row);
        }
        let mut table = builder.build();
        table.with(tabled::Style::psql());
        println!("{table}")
    }
}

fn list_tables(pattern: Option<&str>, connection: &Connection) {
    let mut statement = String::from("SELECT name FROM sqlite_schema WHERE type ='table' AND name NOT LIKE 'sqlite_%'");
    match pattern {
        Some(p) => statement.push_str(format!("AND name LIKE {p};").as_str()),
        None => statement.push(';')
    }
    run_statement(connection, statement)
}

fn run_command(command: &str, args: Vec<&str>, connection: &Connection) {
    match command {
        "quit" => std::process::exit(0),
        "tables" => list_tables(args.get(0).copied(), connection),
        _ => println!("Unknown command '{}'", command)
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let mut rl = DefaultEditor::new()?;

    let mut history = home::home_dir().unwrap_or_default();
    history.push(".libsql_history");
    rl.load_history(history.as_path()).ok();

    println!("libSQL version 0.2.0");
    let connection = match args.db_path.as_deref() {
        None | Some("") | Some(":memory:") => {
            println!("Connected to a transient in-memory database.");
            Connection::open_in_memory()?
        }
        Some(path) => Connection::open(path)?,
    };

    let mut leftovers = String::new();
    loop {
        let prompt = if leftovers.is_empty() {
            "libsql> "
        } else {
            "...   > "
        };
        let readline = rl.readline(prompt);
        match readline {
            Ok(line) => {
                let line = leftovers + line.trim_end();
                if line.ends_with(';') || line.starts_with('.') {
                    leftovers = String::new();
                } else {
                    leftovers = line + " ";
                    continue;
                };
                rl.add_history_entry(&line).ok();
                if line.starts_with('.') {
                    let cmd: String = line[1..].to_string();
                    match cmd.split_once(' ') {
                        Some((command, args)) => run_command(&command, args.split_whitespace().collect(), &connection),
                        None => run_command(&cmd, Vec::new(), &connection)
                    };
                } else {
                    run_statement(&connection, line)
                }
            }
            Err(ReadlineError::Interrupted) => {
                leftovers = String::new();
                continue;
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }
    rl.save_history(history.as_path()).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_str_statements_itterator() {
        let mut str_statements_iterator =
            get_str_statements(String::from("SELECT ';' FROM test; SELECT * FROM test;;"));
        assert_eq!(
            str_statements_iterator.next(),
            Some("SELECT ';' FROM test;".to_owned())
        );
        assert_eq!(
            str_statements_iterator.next(),
            Some("SELECT * FROM test;".to_owned())
        );
        assert_eq!(str_statements_iterator.next(), None);
    }
}
