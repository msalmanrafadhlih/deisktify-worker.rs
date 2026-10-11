//! Mencocokkan klien dengan implementasi referensi resmi.
//!
//! - `req_*.json` dihasilkan encoder `@libsql/hrana-client` (scripts/gen_golden.mjs);
//!   request yang kita susun harus sama persis sebagai JSON.
//! - `resp_*.json` diterima decoder referensi yang sama; di sini kita pastikan
//!   parser kita membaca isinya dengan benar.
//!
//! Batasnya: ini membuktikan bentuk JSON, BUKAN perilaku server Turso sungguhan.

use turso_http::client::transaction_batch;
use turso_http::proto::{
    PipelineRequest, PipelineResponse, StreamRequest, StreamResponse, StreamResult,
};
use turso_http::{Stmt, Value};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn json(name: &str) -> serde_json::Value {
    serde_json::from_str(&fixture(name)).unwrap()
}

fn execute_result(name: &str) -> turso_http::StmtResult {
    let resp: PipelineResponse = serde_json::from_str(&fixture(name)).unwrap();
    match resp.results.into_iter().next().unwrap() {
        StreamResult::Ok {
            response: StreamResponse::Execute { result },
        } => result,
        other => panic!("{other:?}"),
    }
}

#[test]
fn execute_request_matches_reference_encoder() {
    let stmt = Stmt::new("SELECT ?, ?, ?, ?, ?, :name")
        .bind(1_234_567_890_123_i64)
        .bind(2.5)
        .bind("héllo \"dunia\" \\ \n")
        .bind(Value::Null)
        .bind(vec![0u8, 1, 255])
        .bind_named("name", "Turso");
    let req = PipelineRequest {
        baton: None,
        requests: vec![StreamRequest::Execute { stmt }, StreamRequest::Close],
    };
    assert_eq!(
        serde_json::to_value(&req).unwrap(),
        json("req_execute.json")
    );
}

#[test]
fn transaction_request_matches_reference_encoder() {
    let batch = transaction_batch(vec![
        Stmt::new("INSERT INTO t(a) VALUES (?)").bind(1),
        Stmt::new("INSERT INTO t(a) VALUES (?)").bind(2),
    ]);
    let req = PipelineRequest {
        baton: None,
        requests: vec![StreamRequest::Batch { batch }, StreamRequest::Close],
    };
    assert_eq!(
        serde_json::to_value(&req).unwrap(),
        json("req_transaction.json")
    );
}

#[test]
fn select_response_parses_every_value_type() {
    let r = execute_result("resp_select.json");
    assert_eq!(r.cols.len(), 5);
    assert_eq!(r.cols[0].name.as_deref(), Some("i"));
    assert_eq!(r.cols[3].decltype, None);
    // 2^53 + 1: tidak bisa direpresentasikan oleh angka JSON/f64, jadi harus lewat string.
    assert_eq!(r.get(0, "i"), Some(&Value::Integer(9_007_199_254_740_993)));
    assert_eq!(r.get(0, "f"), Some(&Value::Float(2.5)));
    assert_eq!(
        r.get(0, "t").and_then(Value::as_str),
        Some("héllo \"dunia\"")
    );
    assert_eq!(r.get(0, "n"), Some(&Value::Null));
    assert_eq!(
        r.get(0, "b").and_then(Value::as_blob),
        Some(&[0u8, 1, 255][..])
    );
    assert_eq!(r.last_insert_rowid, None);
}

#[test]
fn insert_response_exposes_rowid_and_count() {
    let r = execute_result("resp_insert.json");
    assert_eq!(r.affected_row_count, 1);
    assert_eq!(r.last_insert_rowid, Some(42));
    assert!(r.rows.is_empty());
}

#[test]
fn error_response_parses() {
    let resp: PipelineResponse = serde_json::from_str(&fixture("resp_sql_error.json")).unwrap();
    match &resp.results[0] {
        StreamResult::Error { error } => {
            assert!(error.message.contains("no such table"));
            assert_eq!(error.code.as_deref(), Some("SQL_PARSE_ERROR"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn unknown_response_types_do_not_break_parsing() {
    let body = r#"{"results":[{"type":"ok","response":{"type":"sequence"}},{"type":"ok","response":{"type":"close"}}]}"#;
    let resp: PipelineResponse = serde_json::from_str(body).unwrap();
    assert!(matches!(
        resp.results[0],
        StreamResult::Ok {
            response: StreamResponse::Other
        }
    ));
}

#[test]
fn float_that_cannot_be_json_is_sent_as_null() {
    for f in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            serde_json::to_value(Value::Float(f)).unwrap(),
            serde_json::json!({"type": "null"})
        );
    }
}

#[test]
fn integers_survive_a_round_trip_at_the_extremes() {
    for i in [i64::MIN, -1, 0, 1, i64::MAX] {
        let v = serde_json::to_value(Value::Integer(i)).unwrap();
        assert_eq!(v["value"], serde_json::Value::String(i.to_string()));
        assert_eq!(
            serde_json::from_value::<Value>(v).unwrap(),
            Value::Integer(i)
        );
    }
}

#[test]
fn lenient_integer_decoding_accepts_a_plain_number() {
    let v: Value = serde_json::from_str(r#"{"type":"integer","value":12}"#).unwrap();
    assert_eq!(v, Value::Integer(12));
}

#[test]
fn malformed_values_are_rejected() {
    for bad in [
        r#"{"type":"integer","value":"abc"}"#,
        r#"{"type":"blob","base64":"!!!"}"#,
        r#"{"type":"mystery"}"#,
        r#"{"value":"x"}"#,
    ] {
        assert!(serde_json::from_str::<Value>(bad).is_err(), "{bad}");
    }
}
