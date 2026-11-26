//! BAD ACTOR (run) F8: an oversized host argument is reported as such (not as a missing capability) and a cut log line is marked.
use oxiscript_playground::run::{run_json, DEFAULT_FUEL, LINE_CUT_MARK, MAX_LINE_BYTES};
use serde_json::Value;

const H: &str = "import { Log } from \"host:log\";\n";

fn run(src: &str) -> Value {
    serde_json::from_str(&run_json(src, DEFAULT_FUEL)).unwrap()
}

#[test]
fn oversized_log_message_is_a_bad_argument_not_a_denied_capability() {
    let v = run(&format!("{H}export function main(log: Log): void {{ log.write(2, \"{}\"); }}", "a".repeat(70_000)));
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["stop"]["reason"], "host_bad_argument", "{v}");
    assert_eq!(v["stop"]["code"], "OXS-E0423", "{v}");
}

#[test]
fn long_log_line_is_cut_and_marked() {
    for len in [MAX_LINE_BYTES - 1, MAX_LINE_BYTES] {
        let v = run(&format!("{H}export function main(log: Log): void {{ log.write(2, \"{}\"); }}", "a".repeat(len)));
        assert_eq!(v["lines"][0]["msg"].as_str().unwrap().len(), len, "a line of {len} bytes is shown whole");
    }
    for len in [MAX_LINE_BYTES + 1, 1028, 1029, 2000, 60_000] {
        let v = run(&format!("{H}export function main(log: Log): void {{ log.write(2, \"{}\"); }}", "a".repeat(len)));
        let m = v["lines"][0]["msg"].as_str().unwrap();
        assert!(m.ends_with(LINE_CUT_MARK), "{len}: {}", &m[m.len().saturating_sub(60)..]);
        assert_eq!(m.len(), MAX_LINE_BYTES + LINE_CUT_MARK.len(), "{len}");
        assert!(v["stop"].is_null(), "{v}");
    }
    // multi-byte text is cut on a character boundary
    let v = run(&format!("{H}export function main(log: Log): void {{ log.write(2, \"{}\"); }}", "\u{e9}".repeat(1000)));
    assert!(v["lines"][0]["msg"].as_str().unwrap().ends_with(LINE_CUT_MARK));
}
