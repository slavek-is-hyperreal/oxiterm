#[cfg(test)]
mod tests {
    use std::path::Path;
    use oxiterm_renderer::{CellBuffer, THTMLDocument};
    use oxiterm_renderer::render::renderer::{Renderer, unpremultiply_bgra_to_rgba};
    use oxiterm_proto::dom::{Node, NodeTag};
    use oxiterm_proto::style::TerminalProfile;
    use oxiterm_renderer::layout::{LayoutEngine};

    #[test]
    fn test_23_lottie_load() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let bell_path = manifest_dir.parent().unwrap().join("examples").join("bell.json");
        let contents = std::fs::read_to_string(bell_path).unwrap();
        
        let anim = rlottie::Animation::from_data(contents, "bell".to_string(), "/unused".to_string());
        assert!(anim.is_some());
        let anim = anim.unwrap();
        assert!(anim.totalframe() > 0);
        assert!(anim.framerate() > 0.0);
    }

    #[test]
    fn test_24_lottie_render_frame() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let bell_path = manifest_dir.parent().unwrap().join("examples").join("bell.json");
        let contents = std::fs::read_to_string(bell_path).unwrap();
        
        let mut anim = rlottie::Animation::from_data(contents, "bell".to_string(), "/unused".to_string()).unwrap();
        let mut surface = rlottie::Surface::new(rlottie::Size { width: 64, height: 64 });
        anim.render(0, &mut surface);
        
        let mut has_alpha = false;
        for pixel in surface.data() {
            if pixel.a > 0 {
                has_alpha = true;
                break;
            }
        }
        assert!(has_alpha);
    }

    #[test]
    fn test_25_unpremultiply() {
        let input = vec![
            rlottie::Bgra { b: 0, g: 0, r: 128, a: 128 },
            rlottie::Bgra { b: 0, g: 0, r: 0, a: 0 },
        ];
        let output = unpremultiply_bgra_to_rgba(&input, 2, 1);
        let pixels = output.as_raw();
        // first pixel: Rgba { 255, 0, 0, 128 }
        assert_eq!(pixels[0], 255);
        assert_eq!(pixels[1], 0);
        assert_eq!(pixels[2], 0);
        assert_eq!(pixels[3], 128);
        
        // second pixel: Rgba { 0, 0, 0, 0 }
        assert_eq!(pixels[4], 0);
        assert_eq!(pixels[5], 0);
        assert_eq!(pixels[6], 0);
        assert_eq!(pixels[7], 0);
    }

    #[test]
    fn test_26_full_path_render_node() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let bell_path = manifest_dir.parent().unwrap().join("examples").join("bell.json");
        
        let mut doc = THTMLDocument::new();
        let mut img_node = Node::new(NodeTag::Img);
        img_node.attrs.src = Some(bell_path.to_string_lossy().to_string());
        img_node.style.width = Some(15);
        img_node.style.height = Some(5);
        
        let node_id = doc.arena.alloc(img_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 15, 5, None).unwrap();

        let mut buffer = CellBuffer::new(15, 5);
        let mut profile = TerminalProfile::default();
        profile.supports_kitty_gfx = true;
        
        Renderer::render_node(&doc, &layout, &mut buffer, &profile, None, 0);

        // CellBuffer.graphics should not be empty
        assert!(!buffer.graphics.is_empty());
        // Covered cells should be marked skip
        assert!(buffer.cells[0].skip);
    }
}
