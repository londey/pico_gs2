//! Scenes: they fit the scene ROM, decode to what was encoded, make the
//! triangles their strips and fans should, and render; the cube's winding
//! matches its faces' visibility, so back-face culling draws only the faces
//! the camera sees.

use setup_model::render::{render, Frame, FB_HEIGHT, FB_WIDTH};
use setup_model::scene::{
    cube_frame, cube_scene, decode, draws, encode, test_card, Command, Kind, BACKGROUND,
    CUBE_FRAMES, SCENE_WORDS,
};

/// Encoding then decoding gives a scene back, within the ROM.
#[test]
fn scenes_round_trip() {
    for scene in [draws(test_card()), cube_scene()] {
        let words = encode(&scene);
        assert!(words.len() <= SCENE_WORDS, "{} words", words.len());
        assert_eq!(decode(&words), scene);
    }
}

/// A strip's odd triangles have roles 1 and 2 swapped; a fan's all share
/// its first vertex.
#[test]
fn strips_and_fans_make_their_triangles() {
    for p in test_card() {
        let tris = p.triangles();
        assert_eq!(tris.len(), p.vertices.len() - 2);
        for (k, t) in tris.iter().enumerate() {
            let v = &p.vertices;
            match p.kind {
                Kind::Strip if k % 2 == 0 => assert_eq!(*t, [v[k], v[k + 1], v[k + 2]]),
                Kind::Strip => assert_eq!(*t, [v[k + 1], v[k], v[k + 2]]),
                Kind::Fan => assert_eq!(*t, [v[0], v[k + 1], v[k + 2]]),
            }
        }
    }
}

/// The test card draws most of its triangles and culls some.
#[test]
fn test_card_renders() {
    let mut frame = Frame::new(vec![0x0841_u16; FB_WIDTH * FB_HEIGHT]);
    let total: usize = test_card().iter().map(|p| p.vertices.len() - 2).sum();
    let (_, kept) = render(&mut frame, &encode(&draws(test_card())));
    let drawn = frame.colour.iter().filter(|&&w| w != 0x0841).count();
    println!("test card: {kept} of {total} triangles kept, {drawn} pixels drawn");
    assert!(kept > total / 2 && kept < total, "{kept} of {total} kept");
    assert!(drawn > 20_000, "only {drawn} pixels drawn");
}

/// Every frame of the cube keeps exactly its visible faces (three while
/// the cube is turned, two side faces' worth at worst edge-on) plus the
/// two crossing triangles, and draws over the background.
#[test]
fn cube_frames_render() {
    let scene = cube_scene();
    let mut frame = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
    let (frames, kept) = render(&mut frame, &encode(&scene));
    assert_eq!(frames.len(), CUBE_FRAMES);
    // Per frame: up to three faces of two triangles, and the crossing pair.
    let per_frame = kept as f64 / CUBE_FRAMES as f64;
    println!("cube: {per_frame:.2} triangles kept per frame");
    assert!((6.0..=8.0).contains(&per_frame), "{per_frame} per frame");
    for f in &frames {
        let drawn = f.iter().filter(|&&w| w != BACKGROUND).count();
        assert!(drawn > 20_000, "only {drawn} pixels drawn");
    }
    let draws_per_frame = scene
        .iter()
        .filter(|c| matches!(c, Command::Draw(_)))
        .count();
    assert_eq!(draws_per_frame, CUBE_FRAMES * 8);
}

/// A quarter turn of the cube looks the same as no turn: the loop is
/// seamless (up to the rounding of the turned vertices).
#[test]
fn cube_loops_seamlessly() {
    let image = |k| {
        let mut frame = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
        render(&mut frame, &encode(&cube_frame(k))).0.remove(0)
    };
    let (a, b) = (image(0), image(CUBE_FRAMES));
    let differ = a.iter().zip(&b).filter(|(x, y)| x != y).count();
    println!("a quarter turn on: {differ} pixels differ");
    assert!(differ < 200, "{differ} pixels differ");
}
