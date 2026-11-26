fn push_text(buf: &mut Vec<u8>, s: &str) {
    for b in s.bytes() {
        buf.push(b);
    }
}

fn push_num(buf: &mut Vec<u8>, v: i32) {
    if v >= 10 {
        push_num(buf, v / 10);
    }
    buf.push((48 + v % 10) as u8);
}

pub fn run(n: i32) -> u32 {
    let mut buf: Vec<u8> = Vec::new();
    buf.push(91);
    for i in 0..n {
        if i > 0 {
            buf.push(44);
        }
        push_text(&mut buf, "{\"id\":");
        push_num(&mut buf, i);
        push_text(&mut buf, ",\"name\":\"item");
        push_num(&mut buf, i % 97);
        push_text(&mut buf, "\",\"ok\":true,\"none\":null,\"v\":[1,22,");
        push_num(&mut buf, i * 3);
        push_text(&mut buf, "]}");
    }
    buf.push(93);
    let mut h: u32 = 0;
    let mut tokens: u32 = 0;
    let mut i = 0usize;
    let len = buf.len();
    while i < len {
        let c = buf[i];
        if c == 34 {
            i += 1;
            let start = i;
            while buf[i] != 34 {
                i += 1;
            }
            h = h.wrapping_mul(31).wrapping_add((i - start) as u32);
            i += 1;
            tokens = tokens.wrapping_add(1);
        } else if (48..=57).contains(&c) {
            let mut v: u32 = 0;
            while i < len && (48..=57).contains(&buf[i]) {
                v = v.wrapping_mul(10).wrapping_add((buf[i] - 48) as u32);
                i += 1;
            }
            h = h.wrapping_mul(31).wrapping_add(v);
            tokens = tokens.wrapping_add(1);
        } else if c == 116 {
            i += 4;
            h = h.wrapping_mul(31).wrapping_add(1);
            tokens = tokens.wrapping_add(1);
        } else if c == 110 {
            i += 4;
            h = h.wrapping_mul(31).wrapping_add(2);
            tokens = tokens.wrapping_add(1);
        } else {
            if matches!(c, 123 | 125 | 91 | 93 | 44 | 58) {
                h = h.wrapping_mul(31).wrapping_add(c as u32);
                tokens = tokens.wrapping_add(1);
            }
            i += 1;
        }
    }
    h.wrapping_add(tokens)
}
