pub fn run(n: i32) -> u32 {
    let size = n;
    let (mut sum, mut byte_acc, mut bit_num) = (0i32, 0i32, 0i32);
    for y in 0..size {
        let ci = ((2.0 * y as f64) / size as f64) - 1.0;
        for x in 0..size {
            let (mut zrzr, mut zi, mut zizi) = (0.0f64, 0.0f64, 0.0f64);
            let cr = ((2.0 * x as f64) / size as f64) - 1.5;
            let (mut z, mut not_done, mut escape) = (0, true, 0i32);
            while not_done && z < 50 {
                let zr = zrzr - zizi + cr;
                zi = 2.0 * zr * zi + ci;
                zrzr = zr * zr;
                zizi = zi * zi;
                if zrzr + zizi > 4.0 {
                    not_done = false;
                    escape = 1;
                }
                z += 1;
            }
            byte_acc = (byte_acc << 1) + escape;
            bit_num += 1;
            if bit_num == 8 {
                sum ^= byte_acc;
                byte_acc = 0;
                bit_num = 0;
            } else if x == size - 1 {
                byte_acc <<= 8 - bit_num;
                sum ^= byte_acc;
                byte_acc = 0;
                bit_num = 0;
            }
        }
    }
    sum as u32
}
