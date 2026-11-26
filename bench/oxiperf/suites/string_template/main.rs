pub fn run(n: i32) -> u32 {
    const LABELS: [&str; 4] = ["alpha", "beta", "gamma", "delta"];
    let mut h: u32 = 0;
    let mut total: u32 = 0;
    for i in 0..n {
        let label = LABELS[(i % 4) as usize];
        let x = i * 3;
        let y = (i * 7) % 1000;
        let d = x - y;
        let s = format!("row #{i}: {label} ({x}, {y}) delta={d}");
        for b in s.bytes() {
            h = h.wrapping_mul(31).wrapping_add(b as u32);
        }
        total = total.wrapping_add(s.len() as u32);
    }
    h.wrapping_add(total)
}
