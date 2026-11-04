//! The setup program as an unscheduled list of operations (brief §4.5,
//! with the changes in `docs/design/setup-engine.md`): what each value is
//! computed from, and where it goes, but not when or in which register.

use crate::isa::{field, out, AluOp, Constant, MulOp};
use crate::triangle::{Attr, AttrSet};

/// A value the program computes: an index into [`Program::insts`].
pub type ValueId = usize;

/// An operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Src {
    /// A vertex field, by role (0 to 2) and field code (ALU only).
    Vertex(u8, u8),
    /// A constant (ALU only).
    Const(Constant),
    /// A value computed earlier.
    Value(ValueId),
}

/// One term of a multiplier sum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Term {
    /// MUL for the first term, MAC or MSUB for the others.
    pub op: MulOp,

    /// The 36-bit operand.
    pub a: ValueId,

    /// The 18-bit operand.
    pub b: ValueId,
}

/// How a multiplier sum reaches its register.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Post {
    /// The constant right shift.
    pub shift: u8,

    /// Also shift by `E`.
    pub by_e: bool,

    /// Round half up.
    pub round: bool,

    /// Saturate.
    pub sat: bool,
}

/// What an instruction does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An ALU operation.
    Alu {
        /// The operation.
        op: AluOp,
        /// The first operand.
        a: Src,
        /// The second operand (ignored by one-operand operations).
        b: Src,
    },
    /// A multiplier sum, issued term by term on consecutive cycles.
    Mul {
        /// The terms, in issue order.
        terms: Vec<Term>,
        /// The post-shifter's settings.
        post: Post,
    },
}

/// One instruction: it computes one value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inst {
    /// A name, for reports.
    pub name: String,

    /// What it does.
    pub kind: Kind,

    /// The output slot it also writes, if any.
    pub out: Option<u8>,
}

/// A program: instructions in dependency order, and the TST whose flags
/// the area check uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Program {
    /// The instructions; each reads only values before it.
    pub insts: Vec<Inst>,

    /// The TST of the area, which the CHECK follows.
    pub check_after: ValueId,
}

impl Program {
    /// Appends an ALU instruction.
    fn alu(&mut self, name: &str, op: AluOp, a: Src, b: Src, out: Option<u8>) -> ValueId {
        self.insts.push(Inst {
            name: name.to_string(),
            kind: Kind::Alu { op, a, b },
            out,
        });
        self.insts.len() - 1
    }

    /// Appends a multiplier sum: `a0 * b0`, then each `(op, a, b)`.
    fn mul(
        &mut self,
        name: &str,
        terms: &[(MulOp, ValueId, ValueId)],
        post: Post,
        out: Option<u8>,
    ) -> ValueId {
        self.insts.push(Inst {
            name: name.to_string(),
            kind: Kind::Mul {
                terms: terms.iter().map(|&(op, a, b)| Term { op, a, b }).collect(),
                post,
            },
            out,
        });
        self.insts.len() - 1
    }
}

/// A vertex field.
fn vf(role: u8, f: u8) -> Src {
    Src::Vertex(role, f)
}

/// A computed value.
fn val(v: ValueId) -> Src {
    Src::Value(v)
}

/// The setup program for a set of attributes.
///
/// # Arguments
///
/// * `attrs` - The attributes to compute planes for.
///
/// # Returns
///
/// The unscheduled program.
// One function, in the order of the brief's §4.5, reads best, with the
// brief's names.
#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
pub fn setup_program(attrs: AttrSet) -> Program {
    use AluOp::{Add, Addv, Lzc, Max, Min, Neg, Rcpld, Snap, Sub, Tst};
    use MulOp::{Mac, Msub, Mul};
    let (x, y) = (field::X, field::Y);
    let z = Src::Const(Constant::Zero);
    let mut p = Program::default();

    // Geometry. B01 = dx1 and A20 = dy2 are written as they are computed.
    let dx1 = p.alu("dx1", Sub, vf(1, x), vf(0, x), Some(out::AB + 1));
    let dy1 = p.alu("dy1", Sub, vf(1, y), vf(0, y), None);
    let dx2 = p.alu("dx2", Sub, vf(2, x), vf(0, x), None);
    let dy2 = p.alu("dy2", Sub, vf(2, y), vf(0, y), Some(out::AB + 4));
    let exact = Post {
        shift: 8,
        ..Post::default()
    };
    let area = p.mul("area", &[(Mul, dx1, dy2), (Msub, dx2, dy1)], exact, None);
    p.check_after = p.alu("tst", Tst, val(area), z, None);

    // Bounding box, clipped; the start point snapped to the block grid.
    let xmin01 = p.alu("xmin01", Min, vf(0, x), vf(1, x), None);
    let xs = p.alu("xs", Snap, val(xmin01), vf(2, x), Some(out::BOX));
    let ymin01 = p.alu("ymin01", Min, vf(0, y), vf(1, y), None);
    let ys = p.alu("ys", Snap, val(ymin01), vf(2, y), Some(out::BOX + 1));
    let xmax01 = p.alu("xmax01", Max, vf(0, x), vf(1, x), None);
    let xmax012 = p.alu("xmax012", Max, val(xmax01), vf(2, x), None);
    p.alu(
        "xmax",
        Min,
        val(xmax012),
        Src::Const(Constant::XLimit),
        Some(out::BOX + 2),
    );
    let ymax01 = p.alu("ymax01", Max, vf(0, y), vf(1, y), None);
    let ymax012 = p.alu("ymax012", Max, val(ymax01), vf(2, y), None);
    p.alu(
        "ymax",
        Min,
        val(ymax012),
        Src::Const(Constant::YLimit),
        Some(out::BOX + 3),
    );
    let half = Src::Const(Constant::Half);
    let xc = p.alu("xc", Add, val(xs), half, None);
    let yc = p.alu("yc", Add, val(ys), half, None);
    let c: Vec<(ValueId, ValueId)> = (0..3)
        .map(|role| {
            (
                p.alu(&format!("cx{role}"), Sub, val(xc), vf(role, x), None),
                p.alu(&format!("cy{role}"), Sub, val(yc), vf(role, y), None),
            )
        })
        .collect();

    // Edges: A, B to outputs; E = A cx + B cy on the multiplier.
    p.alu("A01", Neg, val(dy1), z, Some(out::AB));
    let a12 = p.alu("A12", Sub, vf(1, y), vf(2, y), Some(out::AB + 2));
    let b12 = p.alu("B12", Sub, vf(2, x), vf(1, x), Some(out::AB + 3));
    p.alu("B20", Neg, val(dx2), z, Some(out::AB + 5));
    p.mul(
        "E01",
        &[(Mul, dx1, c[0].1), (Msub, dy1, c[0].0)],
        exact,
        Some(out::E),
    );
    p.mul(
        "E12",
        &[(Mul, a12, c[1].0), (Mac, b12, c[1].1)],
        exact,
        Some(out::E + 1),
    );
    p.mul(
        "E20",
        &[(Mul, dy2, c[2].0), (Msub, dx2, c[2].1)],
        exact,
        Some(out::E + 2),
    );

    // 1 / area: r = x0 (2 - m x0).
    let m = p.alu("m", Lzc, val(area), z, None);
    let x0 = p.alu("x0", Rcpld, val(m), z, None);
    let recip = Post {
        shift: 25,
        ..Post::default()
    };
    let t = p.mul("t", &[(Mul, m, x0)], recip, None);
    let t2 = p.alu("t2", Sub, Src::Const(Constant::Two), val(t), None);
    let r = p.mul(
        "r",
        &[(Mul, t2, x0)],
        Post {
            round: true,
            ..recip
        },
        None,
    );

    // Gradient factors k = r d.
    let kpost = Post {
        shift: 22,
        round: true,
        sat: true,
        ..Post::default()
    };
    let k1 = p.mul("k1", &[(Mul, r, dy2)], kpost, None);
    let k2 = p.mul("k2", &[(Mul, r, dy1)], kpost, None);
    let k3 = p.mul("k3", &[(Mul, r, dx1)], kpost, None);
    let k4 = p.mul("k4", &[(Mul, r, dx2)], kpost, None);

    // Per attribute: gradients, then start = a0 + dx cx0 + dy cy0.
    let grad = Post {
        shift: 7,
        by_e: true,
        round: true,
        sat: true,
    };
    let start = Post {
        shift: 12,
        round: true,
        ..Post::default()
    };
    for attr in Attr::ALL {
        if !attrs.contains(attr) {
            continue;
        }
        let i = u8::try_from(attr.index()).expect("8 attributes");
        let f = field::ATTR + i;
        let n = format!("{attr:?}");
        let d1 = p.alu(&format!("{n}.d1"), Sub, vf(1, f), vf(0, f), None);
        let d2 = p.alu(&format!("{n}.d2"), Sub, vf(2, f), vf(0, f), None);
        let gx = p.mul(
            &format!("{n}.dx"),
            &[(Mul, k1, d1), (Msub, k2, d2)],
            grad,
            Some(out::DX + 2 * i),
        );
        let gy = p.mul(
            &format!("{n}.dy"),
            &[(Mul, k3, d2), (Msub, k4, d1)],
            grad,
            Some(out::DX + 2 * i + 1),
        );
        let pv = p.mul(
            &format!("{n}.p"),
            &[(Mul, gx, c[0].0), (Mac, gy, c[0].1)],
            start,
            None,
        );
        p.alu(
            &format!("{n}.start"),
            Addv,
            val(pv),
            vf(0, f),
            Some(out::START + i),
        );
    }
    p
}
