//! Draws a small scene offscreen and checks pixels. The picture is written to `target/ui-test/draw.png` to be looked at.

use nekominal_render::offscreen::{AdapterKind, OffscreenTarget, create_gpu, write_png};
use nekominal_ui::{Color, Rect, Ui};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const SCALE: f32 = 1.5;
const BACKGROUND: Color = Color::rgb(20, 22, 34);
const PANEL: Color = Color::rgb(60, 64, 96);

fn render(draw: impl FnOnce(&mut Ui), size: (u32, u32)) -> Vec<u8> {
    let gpu = create_gpu(AdapterKind::Real).or_else(|_| create_gpu(AdapterKind::Warp)).expect("a DX12 adapter");
    let mut ui = Ui::new(&gpu.device, &gpu.queue, FORMAT).expect("ui");
    let target = OffscreenTarget::new(&gpu.device, size.0, size.1, FORMAT);
    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    // Clear to the background the way the terminal pass would, then draw the UI over it.
    let [r, g, b, _] = BACKGROUND.0.map(|c| f64::from(c) / 255.0);
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &target.view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }), store: wgpu::StoreOp::Store },
        })],
        ..Default::default()
    }));
    ui.begin_frame(size, SCALE);
    draw(&mut ui);
    ui.end_frame(&mut encoder, &target.view);
    target.encode_readback(&mut encoder);
    gpu.queue.submit([encoder.finish()]);
    target.read_rgba(&gpu.device).expect("readback")
}

fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * width + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

#[test]
fn rounded_rectangles_text_and_clips_land_where_they_are_told() {
    let size = (800, 480);
    let mut text_width = 0.0;
    let rgba = render(
        |ui| {
            let panel = Rect::new(20.0, 20.0, 300.0, 120.0);
            ui.shadow(panel, 8.0, 12.0, Color::rgba(0, 0, 0, 160));
            ui.rrect(panel, 8.0, PANEL, Some((Color::rgba(255, 255, 255, 40), 1.0)));
            text_width = ui.text(36.0, 32.0, "終端機 分頁 12 / 30 Nekominal", 14.0, Color::rgb(235, 235, 245), false);
            ui.text(36.0, 56.0, "Bold text 設定", 14.0, Color::rgb(235, 235, 245), true);
            ui.icon(36.0, 84.0, '\u{E713}', 20.0, Color::rgb(130, 170, 255));
            ui.text(70.0, 86.0, "😀 emoji", 14.0, Color::rgb(235, 235, 245), false);
            // Text clipped to a 40 DIP wide box must not reach beyond it.
            ui.push_clip(Rect::new(340.0, 20.0, 40.0, 30.0));
            ui.text(340.0, 24.0, "clipped clipped clipped", 14.0, Color::rgb(255, 255, 255), false);
            ui.pop_clip();
        },
        size,
    );
    std::fs::create_dir_all("../../target/ui-test").unwrap();
    write_png(std::path::Path::new("../../target/ui-test/draw.png"), size.0, size.1, &rgba).unwrap();

    let at = |x: f32, y: f32| pixel(&rgba, size.0, (x * SCALE) as u32, (y * SCALE) as u32);
    // The middle of the panel carries its fill, a point far from everything the background.
    let inside = at(250.0, 120.0);
    assert!(inside.iter().zip(PANEL.0).all(|(a, b)| a.abs_diff(b) <= 3), "panel fill, got {inside:?}");
    assert_eq!(at(500.0, 250.0), [BACKGROUND.0[0], BACKGROUND.0[1], BACKGROUND.0[2]]);
    // The very corner is rounded: it shows the shadow over the background, not the fill.
    let corner = at(20.2, 20.2);
    assert!(corner.iter().zip(PANEL.0).any(|(a, b)| a.abs_diff(b) > 12), "the corner is rounded, got {corner:?}");
    assert!(text_width > 100.0 && text_width < 300.0, "text width {text_width}");
    // Something lighter than the panel was drawn where the text is.
    let lit = (36..200).flat_map(|x| (32..48).map(move |y| (x, y))).filter(|&(x, y)| at(x as f32, y as f32)[0] > 150).count();
    assert!(lit > 40, "{lit} text pixels");
    // The clip: ink inside the box, nothing to the right of it.
    let right_of_clip = (381..460).flat_map(|x| (20..50).map(move |y| (x, y))).filter(|&(x, y)| at(x as f32, y as f32)[0] > 100).count();
    assert_eq!(right_of_clip, 0, "text leaked out of its clip");
    let in_clip = (340..380).flat_map(|x| (24..44).map(move |y| (x, y))).filter(|&(x, y)| at(x as f32, y as f32)[0] > 100).count();
    assert!(in_clip > 10, "no text inside the clip");
}

#[test]
fn text_is_measured_the_way_it_is_drawn() {
    let gpu = create_gpu(AdapterKind::Real).or_else(|_| create_gpu(AdapterKind::Warp)).expect("a DX12 adapter");
    let mut ui = Ui::new(&gpu.device, &gpu.queue, FORMAT).expect("ui");
    ui.begin_frame((100, 100), SCALE);
    let measured = ui.measure("Nekominal 設定", 14.0, false);
    let drawn = ui.text(0.0, 0.0, "Nekominal 設定", 14.0, Color::rgb(255, 255, 255), false);
    assert!((measured - drawn).abs() < 0.01, "{measured} vs {drawn}");
    assert!(ui.measure("", 14.0, false) == 0.0);
    assert!(ui.measure("wide 設定", 14.0, false) > ui.measure("wide", 14.0, false));
    assert!(ui.line_height(14.0) > 14.0);
}
