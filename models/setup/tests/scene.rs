//! Scenes: they become transactions that fit the scene ROM, which the
//! command parser's interpretation turns back into exactly the scene's
//! triangles and frame operations; strips and fans make the triangles they
//! should; scenes render; the cube's winding matches its faces' visibility,
//! so back-face culling draws only the faces the camera sees.

use gs_proto::{reg, Attr0, Frame as FrameReg, Position, RenderMode, Transaction};
use setup_model::render::{render, Frame, FB_HEIGHT, FB_WIDTH};
use setup_model::scene::{
    cube_frame, cube_scene, draws, interpret, rom, test_card, transactions, Command, Event, Kind,
    BACKGROUND, CUBE_FRAMES, ROM_END, SCENE_ENTRIES,
};

/// What the parser should make of a scene: each primitive's triangles,
/// each clear and end of frame (an end of frame and the next clear as one).
fn expected_events(cmds: &[Command]) -> Vec<Event> {
    let transactions_frame = |present, clear: Option<u16>| {
        Event::Frame(FrameReg {
            present,
            clear_colour: clear.is_some(),
            clear_depth: clear.is_some(),
            colour: clear.unwrap_or(0),
        })
    };
    let mut events = Vec::new();
    let mut cmds = cmds.iter().peekable();
    while let Some(c) = cmds.next() {
        match c {
            Command::Clear(rgb) => events.push(transactions_frame(false, Some(*rgb))),
            Command::EndFrame => {
                if let Some(Command::Clear(rgb)) = cmds.peek() {
                    events.push(transactions_frame(true, Some(*rgb)));
                    cmds.next();
                } else {
                    events.push(transactions_frame(true, None));
                }
            }
            Command::Draw(p) => {
                let mode = interpret(&transactions(std::slice::from_ref(c)))
                    .into_iter()
                    .find_map(|e| match e {
                        Event::Draw(_, m) => Some(m),
                        Event::Frame(_) => None,
                    })
                    .expect("a primitive draws");
                events.extend(p.triangles().into_iter().map(|t| Event::Draw(t, mode)));
            }
        }
    }
    events
}

/// Interpreting a scene's transactions gives back its triangles, in role
/// order, and its frame operations.
#[test]
fn transactions_interpret_to_the_scene() {
    for scene in [draws(test_card()), cube_scene()] {
        assert_eq!(interpret(&transactions(&scene)), expected_events(&scene));
    }
}

/// The cube fits the scene ROM, ending with the end marker, and its
/// transactions are mostly vertices.
#[test]
fn cube_fits_the_rom() {
    let txns = transactions(&cube_scene());
    let entries = rom(&cube_scene());
    assert_eq!(entries.len(), SCENE_ENTRIES);
    assert_eq!(entries[txns.len()], (ROM_END, 0));
    let per_frame = txns.len() as f64 / CUBE_FRAMES as f64;
    println!(
        "cube: {} transactions, {per_frame:.1} a frame, {} free",
        txns.len(),
        SCENE_ENTRIES - txns.len()
    );
    assert!(per_frame < 60.0, "{per_frame} a frame");
}

/// A strip's odd triangles are taken (two back, this one, one back); a
/// fan's all share its first vertex.
#[test]
fn strips_and_fans_make_their_triangles() {
    for p in test_card() {
        let tris = p.triangles();
        assert_eq!(tris.len(), p.vertices.len() - 2);
        for (k, t) in tris.iter().enumerate() {
            let v = &p.vertices;
            match p.kind {
                Kind::Strip if k % 2 == 0 => assert_eq!(*t, [v[k], v[k + 1], v[k + 2]]),
                Kind::Strip => assert_eq!(*t, [v[k], v[k + 2], v[k + 1]]),
                Kind::Fan => assert_eq!(*t, [v[0], v[k + 1], v[k + 2]]),
            }
        }
    }
}

/// The parser's rules on hand-written transactions: `ATTR0` is latched; a
/// kick before a primitive's third vertex draws nothing; a `NOKICK` after a
/// kick starts a new primitive, which a fan pivots on; reads draw nothing.
#[test]
fn kicks_follow_the_rules() {
    let red = Attr0 {
        r: 255,
        ..Attr0::default()
    };
    let at = |x: i16, y: i16| Position {
        x: x * 16,
        y: y * 16,
        z: 100,
        q: 0,
    };
    let w = |addr: u8, data: u64| Transaction::write(addr, data);
    let txns = [
        w(reg::ATTR0, red.bits()),
        w(reg::VERTEX_NOKICK, at(0, 0).bits()),
        // A kick on the second vertex: too early to draw.
        w(reg::VERTEX_KICK_012, at(10, 0).bits()),
        w(reg::VERTEX_KICK_012, at(0, 10).bits()),
        Transaction::Read { addr: reg::ID },
        // A new primitive, a fan.
        w(reg::VERTEX_NOKICK, at(50, 50).bits()),
        w(reg::VERTEX_NOKICK, at(60, 50).bits()),
        w(reg::VERTEX_KICK_FAN, at(60, 60).bits()),
        w(reg::VERTEX_KICK_FAN, at(50, 60).bits()),
    ];
    let tris: Vec<_> = interpret(&txns)
        .into_iter()
        .map(|e| match e {
            Event::Draw(t, m) => {
                assert_eq!(m, RenderMode::default());
                assert!(t.iter().all(|v| v.r == 255 && v.z == 100));
                t.map(|v| (v.x.to_count() / 16, v.y.to_count() / 16))
            }
            Event::Frame(_) => panic!("no frame writes"),
        })
        .collect();
    assert_eq!(
        tris,
        [
            [(0, 0), (10, 0), (0, 10)],
            [(50, 50), (60, 50), (60, 60)],
            [(50, 50), (60, 60), (50, 60)],
        ]
    );
}

/// The test card draws most of its triangles and culls some.
#[test]
fn test_card_renders() {
    let mut frame = Frame::new(vec![0x0841_u16; FB_WIDTH * FB_HEIGHT]);
    let total: usize = test_card().iter().map(|p| p.vertices.len() - 2).sum();
    let (_, kept) = render(&mut frame, &transactions(&draws(test_card())));
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
    let (frames, kept) = render(&mut frame, &transactions(&scene));
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
        render(&mut frame, &transactions(&cube_frame(k)))
            .0
            .remove(0)
    };
    let (a, b) = (image(0), image(CUBE_FRAMES));
    let differ = a.iter().zip(&b).filter(|(x, y)| x != y).count();
    println!("a quarter turn on: {differ} pixels differ");
    assert!(differ < 200, "{differ} pixels differ");
}

/// With the depth test off, the later of two overlapping triangles wins;
/// with colour writes off, nothing changes colour.
#[test]
fn render_mode_controls_the_writes() {
    let tri = |x: i16, z: u16, r: u8| -> Vec<Transaction> {
        let at = |x: i16, y: i16| Position {
            x: x * 16,
            y: y * 16,
            z,
            q: 0,
        };
        vec![
            Transaction::write(
                reg::ATTR0,
                Attr0 {
                    r,
                    ..Attr0::default()
                }
                .bits(),
            ),
            Transaction::write(reg::VERTEX_NOKICK, at(x, 10).bits()),
            Transaction::write(reg::VERTEX_NOKICK, at(x + 100, 10).bits()),
            Transaction::write(reg::VERTEX_KICK_012, at(x, 110).bits()),
        ]
    };
    let mode = |m: RenderMode| Transaction::write(reg::RENDER_MODE, m.bits());
    let pixel = |txns: &[Transaction]| {
        let mut frame = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
        render(&mut frame, txns);
        frame.colour[20 * FB_WIDTH + 20] >> 11
    };
    // Near (z 100) and red first, then far (z 9000) and almost black.
    let mut near_then_far = tri(10, 100, 255);
    near_then_far.extend(tri(10, 9000, 8));
    assert_eq!(pixel(&near_then_far), 31, "the nearer one stays");
    let mut no_test = vec![mode(RenderMode {
        z_test: false,
        ..RenderMode::default()
    })];
    no_test.extend(near_then_far.clone());
    assert_eq!(pixel(&no_test), 1, "the later one wins");
    let mut no_colour = vec![mode(RenderMode {
        colour_write: false,
        ..RenderMode::default()
    })];
    no_colour.extend(near_then_far);
    assert_eq!(pixel(&no_colour), 0, "nothing drawn");
}
