pub fn run(n: i32) -> u32 {
    let mut total: u32 = 0;
    for _ in 0..n {
        let mut flags = vec![true; 5000];
        let mut prime_count: i32 = 0;
        for i in 2..=5000usize {
            if flags[i - 1] {
                prime_count += 1;
                let mut k = i + i;
                while k <= 5000 {
                    flags[k - 1] = false;
                    k += i;
                }
            }
        }
        total = total.wrapping_add(prime_count as u32);
    }
    total
}
