use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};
use std::thread;

/// Represents a value entry in the database with an optional expiration timestamp.
#[derive(Clone)]
struct DBValEntry {
    value: String,
    expires_at: Option<SystemTime>,
}

impl DBValEntry {
    fn is_expired(&self) -> bool {
        if let Some(expiry) = self.expires_at {
            SystemTime::now() >= expiry
        } else {
            false
        }
    }
}

/// Represents our database engine managing key-value storage with TTL support.
struct KauDB {
    store: RwLock<HashMap<String, DBValEntry>>,
}

impl KauDB {
    fn new() -> Self {
        Self {
            store: RwLock::new(HashMap::new()),
        }
    }

    fn set(&self, key: String, value: String, ttl_seconds: Option<u64>) {
        let expires_at = ttl_seconds.map(|secs| SystemTime::now() + Duration::from_secs(secs));
        let entry = DBValEntry { value, expires_at };
        
        let mut store = self.store.write().unwrap();
        store.insert(key, entry);
    }

    fn get(&self, key: &str) -> Option<String> {
        {
            let store = self.store.read().unwrap();
            if let Some(entry) = store.get(key) {
                if !entry.is_expired() {
                    return Some(entry.value.clone());
                }
            } else {
                return None;
            }
        }

        // Lazy Deletion fallback
        let mut store = self.store.write().unwrap();
        if let Some(entry) = store.get(key) {
            if entry.is_expired() {
                store.remove(key);
            }
        }
        None
    }

    fn delete(&self, key: &str) -> Option<String> {
        let mut store = self.store.write().unwrap();
        if let Some(entry) = store.remove(key) {
            if !entry.is_expired() {
                return Some(entry.value);
            }
        }
        None
    }

    fn list(&self) -> Vec<(String, String)> {
        let store = self.store.read().unwrap();
        let now = SystemTime::now();
        
        store.iter()
            .filter(|(_, entry)| {
                if let Some(expiry) = entry.expires_at {
                    now < expiry
                } else {
                    true
                }
            })
            .map(|(k, entry)| (k.clone(), entry.value.clone()))
            .collect()
    }

    /// Background task method to actively purge expired keys from memory
    fn purge_expired(&self) {
        let mut store = self.store.write().unwrap();
        let initial_size = store.len();
        
        // Remove items where is_expired() evaluates to true
        store.retain(|_, entry| !entry.is_expired());
        
        let purged_count = initial_size - store.len();
        if purged_count > 0 {
            // Optional: Print to terminal when background purges happen
            // In a production app, you would use a logging crate like `log` or `tracing`
            println!("\n[Background Worker] Actively purged {} expired key(s).", purged_count);
        }
    }
}

fn main() {
    // Wrap database in an Arc (Atomic Reference Counter) to safely share it across threads
    let db = Arc::new(KauDB::new());
    let mut rl = DefaultEditor::new().expect("Failed to initialize CLI reader");

    // Clone the Arc pointer for the background purging thread
    let db_clone = Arc::clone(&db);
    thread::spawn(move || {
        loop {
            // Wake up every 5 seconds to clean up memory
            thread::sleep(Duration::from_secs(5));
            db_clone.purge_expired();
        }
    });

    println!("=========================================");
    println!("  Welcome to KauDB!");
    println!("  Available commands:                     ");
    println!("    SET <key> <value> [ttl_seconds]       ");
    println!("    GET <key>                             ");
    println!("    DEL <key>                             ");
    println!("    LIST                                  ");
    println!("    EXIT                                  ");
    println!("=========================================");

    loop {
        let readline = rl.readline("kaudb> ");
        match readline {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                let command = parts[0].to_uppercase();
                let _ = rl.add_history_entry(trimmed);

                match command.as_str() {
                    "SET" => {
                        if parts.len() < 3 {
                            println!("Error: SET syntax is 'SET <key> <value> [ttl_seconds]'");
                            continue;
                        }
                        
                        let key = parts[1].to_string();
                        let mut value_parts = parts[2..].to_vec();
                        let mut ttl_seconds: Option<u64> = None;

                        if value_parts.len() > 1 {
                            if let Some(last_word) = value_parts.last() {
                                if let Ok(parsed_ttl) = last_word.parse::<u64>() {
                                    ttl_seconds = Some(parsed_ttl);
                                    value_parts.pop();
                                }
                            }
                        }

                        let value = value_parts.join(" ");
                        db.set(key.clone(), value, ttl_seconds);
                        
                        match ttl_seconds {
                            Some(secs) => println!("OK -> {} stored with {}s TTL.", key, secs),
                            None => println!("OK -> {} stored persistently.", key),
                        }
                    }
                    "GET" => {
                        if parts.len() != 2 {
                            println!("Error: GET syntax is 'GET <key>'");
                            continue;
                        }
                        match db.get(parts[1]) {
                            Some(val) => println!("\"{}\"", val),
                            None => println!("(nil)"),
                        }
                    }
                    "DEL" => {
                        if parts.len() != 2 {
                            println!("Error: DEL syntax is 'DEL <key>'");
                            continue;
                        }
                        match db.delete(parts[1]) {
                            Some(_) => println!("OK -> Key deleted."),
                            None => println!("(nil) - Key not found or expired."),
                        }
                    }
                    "LIST" => {
                        let records = db.list();
                        if records.is_empty() {
                            println!("(empty store)");
                        } else {
                            for (k, v) in records {
                                println!("{}: \"{}\"", k, v);
                            }
                        }
                    }
                    "EXIT" | "QUIT" => {
                        println!("Goodbye!");
                        break;
                    }
                    _ => {
                        println!("Unknown command: '{}'. Try SET, GET, DEL, LIST, or EXIT.", parts[0]);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("Ctrl-C caught. Exiting...");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("Ctrl-D caught. Exiting...");
                break;
            }
            Err(err) => {
                println!("Error reading input: {:?}", err);
                break;
            }
        }
    }
}
