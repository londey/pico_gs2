//! The test card: it fits the scene ROM, decodes to what was encoded, makes
//! the triangles its strips and fans should, and renders.

use setup_model::render::{render, FB_HEIGHT, FB_WIDTH};
use setup_model::scene::{decode, encode, test_card, Kind, SCENE_WORDS};

/// Encoding then decoding gives the scene back, within the ROM.
#[test]
fn test_card_round_trips() {
    let card = test_card();
    let words = encode(&card);
    assert!(words.len() <= SCENE_WORDS);
    assert_eq!(decode(&words), card);
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

/// The test card draws most of its triangles, culls some, and changes
/// pixels.
#[test]
fn test_card_renders() {
    let mut fb = vec![0x0841_u16; FB_WIDTH * FB_HEIGHT];
    let total: usize = test_card().iter().map(|p| p.vertices.len() - 2).sum();
    let kept = render(&mut fb, &encode(&test_card()));
    let drawn = fb.iter().filter(|&&w| w != 0x0841).count();
    println!("test card: {kept} of {total} triangles kept, {drawn} pixels drawn");
    assert!(kept > total / 2 && kept < total, "{kept} of {total} kept");
    assert!(drawn > 20_000, "only {drawn} pixels drawn");
}
