//! Native Rust baseline: the Rust twins of the workloads, compiled into this binary
//! (`cargo` release profile: opt-level 3).

#[path = "../../../bench/oxiperf/suites/awfy_mandelbrot/main.rs"]
#[allow(clippy::pedantic, clippy::nursery)] // benchmark twin: kept as written, identical across languages
mod awfy_mandelbrot;
#[path = "../../../bench/oxiperf/suites/awfy_sieve/main.rs"]
#[allow(clippy::pedantic, clippy::nursery)] // benchmark twin: kept as written, identical across languages
mod awfy_sieve;
#[path = "../../../bench/oxiperf/suites/sort/main.rs"]
#[allow(clippy::pedantic, clippy::nursery)] // benchmark twin: kept as written, identical across languages
mod sort;
#[path = "../../../bench/oxiperf/suites/map_set/main.rs"]
#[allow(clippy::pedantic, clippy::nursery)] // benchmark twin: kept as written, identical across languages
mod map_set;
#[path = "../../../bench/oxiperf/suites/string_template/main.rs"]
#[allow(clippy::pedantic, clippy::nursery)] // benchmark twin: kept as written, identical across languages
mod string_template;
#[path = "../../../bench/oxiperf/suites/json_tokenize/main.rs"]
#[allow(clippy::pedantic, clippy::nursery)] // benchmark twin: kept as written, identical across languages
mod json_tokenize;
#[path = "../../../bench/oxiperf/suites/tree/main.rs"]
#[allow(clippy::pedantic, clippy::nursery)] // benchmark twin: kept as written, identical across languages
mod tree;
#[path = "../../../bench/oxiperf/suites/matmul/main.rs"]
#[allow(clippy::pedantic, clippy::nursery)] // benchmark twin: kept as written, identical across languages
mod matmul;

pub fn run(id: &str, n: i32) -> Option<u32> {
    Some(match id {
        "awfy_mandelbrot" => awfy_mandelbrot::run(n),
        "awfy_sieve" => awfy_sieve::run(n),
        "sort" => sort::run(n),
        "map_set" => map_set::run(n),
        "string_template" => string_template::run(n),
        "json_tokenize" => json_tokenize::run(n),
        "tree" => tree::run(n),
        "matmul" => matmul::run(n),
        _ => return None,
    })
}
