//! The fixtures harness (impl plan §4 step 6): every `fixtures/*.json`
//! is a (types, shape, canonical, accepts, rejects) bundle. These files
//! are the shared goldens that later become differential-test inputs
//! for `soilc`'s passes.
//!
//! For each fixture: `canonical` must decode and re-encode byte-equal
//! (when present — opaque types have no decodable canonical form);
//! every `accepts` entry must decode to a value `eq` to (and hashing
//! equal to) the canonical value; every `rejects` entry must fail to
//! decode.

use serde_json::Value as JsonValue;
use soil_rt::{json, ops, Runtime};

#[test]
fn fixtures_conform() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("fixtures directory exists") {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }
        run_fixture(&path);
        checked += 1;
    }
    assert!(checked >= 15, "expected the full corpus, found {checked}");
}

fn run_fixture(path: &std::path::Path) {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let raw = std::fs::read_to_string(path).unwrap();
    let fixture: JsonValue = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{name}: fixture is not valid JSON: {e}"));

    let mut rt = Runtime::new();
    let types = fixture["types"].to_string();
    soil_rt::load_descriptors(&mut rt, &types)
        .unwrap_or_else(|e| panic!("{name}: types failed to load: {e}"));
    let shape = soil_rt::descriptor_json::parse_shape_json(&rt, &fixture["type"].to_string())
        .unwrap_or_else(|e| panic!("{name}: bad shape: {e}"));

    let canonical_value = match &fixture["canonical"] {
        JsonValue::Null => None,
        JsonValue::String(canonical) => {
            let value = json::decode(&rt, &shape, canonical)
                .unwrap_or_else(|e| panic!("{name}: canonical failed to decode: {e}"));
            let reencoded = json::encode(&rt, &value)
                .unwrap_or_else(|e| panic!("{name}: canonical failed to re-encode: {e}"));
            assert_eq!(
                &reencoded, canonical,
                "{name}: canonical round trip is not byte-equal"
            );
            Some(value)
        }
        other => panic!("{name}: canonical must be a string or null, found {other}"),
    };

    for (i, accept) in fixture["accepts"].as_array().unwrap().iter().enumerate() {
        let accept = accept.as_str().unwrap();
        let value = json::decode(&rt, &shape, accept)
            .unwrap_or_else(|e| panic!("{name}: accepts[{i}] failed to decode: {e}"));
        let canonical_value = canonical_value
            .as_ref()
            .unwrap_or_else(|| panic!("{name}: accepts requires a canonical"));
        assert!(
            ops::eq(&rt, &value, canonical_value).unwrap(),
            "{name}: accepts[{i}] decoded to a different value"
        );
        assert_eq!(
            ops::hash(&rt, &value).unwrap(),
            ops::hash(&rt, canonical_value).unwrap(),
            "{name}: accepts[{i}] hashes differently"
        );
    }

    for (i, reject) in fixture["rejects"].as_array().unwrap().iter().enumerate() {
        let reject = reject.as_str().unwrap();
        assert!(
            json::decode(&rt, &shape, reject).is_err(),
            "{name}: rejects[{i}] was accepted: {reject}"
        );
    }
}
