struct Node {
    left: Option<Box<Node>>,
    right: Option<Box<Node>>,
}

fn make(d: i32) -> Node {
    if d == 0 {
        Node { left: None, right: None }
    } else {
        Node { left: Some(Box::new(make(d - 1))), right: Some(Box::new(make(d - 1))) }
    }
}

fn count(n: &Node) -> i32 {
    let mut c = 1;
    if let Some(l) = &n.left {
        c += count(l);
    }
    if let Some(r) = &n.right {
        c += count(r);
    }
    c
}

pub fn run(n: i32) -> u32 {
    let mut h: u32 = 0;
    for rep in 0..8 {
        let t = make(n);
        h = h.wrapping_mul(31).wrapping_add((count(&t) + rep) as u32);
    }
    h
}
