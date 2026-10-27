//! Stands in for the GitHub CLI during end-to-end tests: answers the two
//! `gh auth` calls ght makes, using the accounts in `GHT_FIXTURE`.

use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = std::env::var("GHT_FIXTURE").expect("GHT_FIXTURE must point at the fixture file");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("read fixture")).expect("fixture is JSON");
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["auth", "status", "--json", "hosts"] => {
            let hosts: Vec<Value> = fixture["accounts"]
                .as_array()
                .expect("accounts")
                .iter()
                .map(|a| json!({ "state": "success", "active": a["active"], "host": "github.com", "login": a["login"] }))
                .collect();
            println!("{}", json!({ "hosts": { "github.com": hosts } }));
        }
        ["auth", "token", "--user", user] => println!("token-{user}"),
        _ => {
            eprintln!("fake gh: unsupported arguments {args:?}");
            std::process::exit(1);
        }
    }
}
