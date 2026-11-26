fn merge_sort(a: &mut [i32], tmp: &mut [i32], lo: usize, hi: usize) {
    if hi - lo < 2 {
        return;
    }
    let mid = lo + (hi - lo) / 2;
    merge_sort(a, tmp, lo, mid);
    merge_sort(a, tmp, mid, hi);
    let (mut i, mut j, mut k) = (lo, mid, lo);
    while i < mid && j < hi {
        if a[i] <= a[j] {
            tmp[k] = a[i];
            i += 1;
        } else {
            tmp[k] = a[j];
            j += 1;
        }
        k += 1;
    }
    while i < mid {
        tmp[k] = a[i];
        i += 1;
        k += 1;
    }
    while j < hi {
        tmp[k] = a[j];
        j += 1;
        k += 1;
    }
    a[lo..hi].copy_from_slice(&tmp[lo..hi]);
}

pub fn run(n: i32) -> u32 {
    let n = n as usize;
    let mut s: u32 = 2463534242;
    let mut a = vec![0i32; n];
    let mut tmp = vec![0i32; n];
    for x in a.iter_mut() {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        *x = (s >> 1) as i32;
    }
    merge_sort(&mut a, &mut tmp, 0, n);
    let mut h: u32 = 0;
    for x in &a {
        h = h.wrapping_mul(31).wrapping_add(*x as u32);
    }
    h
}
