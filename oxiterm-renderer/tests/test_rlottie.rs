#[cfg(test)]
mod tests {
    #[test]
    fn test_rlottie_api() {
        let anim_opt = rlottie::Animation::from_data(String::new(), String::new(), String::new());
        if let Some(mut anim) = anim_opt {
            let size = anim.size();
            let mut surface = rlottie::Surface::new(size);
            anim.render(0, &mut surface);
            for pixel in surface.data() {
                let _r = pixel.r;
                let _g = pixel.g;
                let _b = pixel.b;
                let _a = pixel.a;
            }
        }
    }
}
