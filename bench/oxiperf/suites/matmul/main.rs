pub fn run(n: i32) -> u32 {
    let n = n as usize;
    let mut a = vec![0f64; n * n];
    let mut b = vec![0f64; n * n];
    let mut p = 0;
    for i in 0..n {
        for j in 0..n {
            a[p] = ((i * 31 + j * 17) % 13) as f64 / 4.0;
            b[p] = ((i * 7 + j * 29) % 11) as f64 / 4.0;
            p += 1;
        }
    }
    let mut c = vec![0f64; n * n];
    p = 0;
    for i in 0..n {
        for j in 0..n {
            let mut acc = 0.0f64;
            for k in 0..n {
                acc += a[i * n + k] * b[k * n + j];
            }
            c[p] = acc;
            p += 1;
        }
    }
    let mut total = 0.0f64;
    for x in &c {
        total += *x;
    }
    (total * 16.0) as u32
}
