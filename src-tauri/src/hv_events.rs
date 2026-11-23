//! Hunsterverse: the events running now, for the launcher's event badges.
//!
//! The wrapper's `/v2/altclient/stats` builds its event flags from Erupe's old
//! config (Festa/Diva/Tournament overrides, EarthStatus) and the `events`
//! table, which on our server only holds the Hunter Festa. Our own events
//! (Speedrun Cup, VS Bracket, Diva Defense, campaigns, Sky Corridor, Raviente,
//! MezFes every day, the Road vote) are in the REST API, as the website's
//! Events page reads them (js/events.js). The launcher reads the same two
//! public answers; src/hvEvents.js turns them into badges, and falls back to
//! the wrapper's flags when they fail.

use serde_json::{json, Value};

/// Schedule rows of the events (speedrun_cup, vs_bracket, diva, campaigns).
pub const CURRENT_PATH: &str = "/api/public/events/current";
/// The Hunter Festival and its phase.
pub const FESTA_PATH: &str = "/api/public/festa";

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let res = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("{url}: HTTP {}", res.status()));
    }
    res.json::<Value>().await.map_err(|e| e.to_string())
}

/// `{"current": <events/current>, "festa": <festa>}`; a part that failed is
/// null. An error only when both failed.
pub async fn fetch(client: &reqwest::Client, api: &str) -> Result<Value, String> {
    let current_url = format!("{api}{CURRENT_PATH}");
    let festa_url = format!("{api}{FESTA_PATH}");
    let (current, festa) = tokio::join!(
        get_json(client, &current_url),
        get_json(client, &festa_url)
    );
    combine(current, festa)
}

fn combine(current: Result<Value, String>, festa: Result<Value, String>) -> Result<Value, String> {
    match (current, festa) {
        (Err(a), Err(_)) => Err(a),
        (current, festa) => Ok(json!({
            "current": current.unwrap_or(Value::Null),
            "festa": festa.unwrap_or(Value::Null),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_failed_part_is_null() {
        let ok = combine(Ok(json!({"events": []})), Err("down".into())).unwrap();
        assert_eq!(ok["current"], json!({"events": []}));
        assert!(ok["festa"].is_null());
        let ok = combine(Err("down".into()), Ok(json!({"festival": null}))).unwrap();
        assert!(ok["current"].is_null());
        assert!(combine(Err("a".into()), Err("b".into())).is_err());
    }
}
