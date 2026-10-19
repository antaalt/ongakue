//! Loop guards: a visual's code is user input, and a loop that never ends
//! would hang the GPU until the system resets it. Every loop of a shader is
//! made to stop once the pixel has run [`MAX_ITERATIONS`] loop iterations in
//! all, counting the iterations of all its loops, so nested loops can't
//! multiply it.
//!
//! The guard is added to naga's representation of the shader, where all
//! loops (WGSL's `loop`, `for` and `while`, and GLSL's) are the same
//! statement, at the start of each loop's body:
//!
//! ```wgsl
//! loop_iterations += 1u;
//! if loop_iterations > MAX_ITERATIONS { break; }
//! ```

use naga::{
    AddressSpace, Arena, BinaryOperator, Block, Expression, Function, GlobalVariable, Handle,
    Literal, MemoryDecorations, Module, Range, Scalar, Span, Statement, Type, TypeInner,
};

/// Most loop iterations a pixel may run, in all its loops. Well above what
/// real shaders use (ray marchers take a few hundred steps).
pub const MAX_ITERATIONS: u32 = 100_000;

/// Adds the guard to every loop of the module.
pub fn guard_loops(module: &mut Module) {
    let u32_type = module.types.insert(
        Type {
            name: None,
            inner: TypeInner::Scalar(Scalar::U32),
        },
        Span::UNDEFINED,
    );
    // Private: one per pixel (fragment shader invocation), starting at 0.
    let counter = module.global_variables.append(
        GlobalVariable {
            name: Some("loop_iterations".to_owned()),
            space: AddressSpace::Private,
            binding: None,
            ty: u32_type,
            init: None,
            memory_decorations: MemoryDecorations::empty(),
        },
        Span::UNDEFINED,
    );
    for (_, function) in module.functions.iter_mut() {
        guard_function(function, counter);
    }
    for entry_point in &mut module.entry_points {
        guard_function(&mut entry_point.function, counter);
    }
}

fn guard_function(function: &mut Function, counter: Handle<GlobalVariable>) {
    guard_block(&mut function.body, &mut function.expressions, counter);
}

/// Guards the loops of a block, including the nested ones.
fn guard_block(
    block: &mut Block,
    expressions: &mut Arena<Expression>,
    counter: Handle<GlobalVariable>,
) {
    for (statement, _) in block.span_iter_mut() {
        match statement {
            Statement::Block(inner) => guard_block(inner, expressions, counter),
            Statement::If { accept, reject, .. } => {
                guard_block(accept, expressions, counter);
                guard_block(reject, expressions, counter);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    guard_block(&mut case.body, expressions, counter);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                guard_block(body, expressions, counter);
                guard_block(continuing, expressions, counter);
                let rest = std::mem::take(body);
                *body = loop_guard(expressions, counter);
                body.extend_block(rest);
            }
            _ => {}
        }
    }
}

/// The statements counting an iteration, and leaving the loop past the limit.
fn loop_guard(expressions: &mut Arena<Expression>, counter: Handle<GlobalVariable>) -> Block {
    let mut add = |expression| expressions.append(expression, Span::UNDEFINED);
    let pointer = add(Expression::GlobalVariable(counter));
    let one = add(Expression::Literal(Literal::U32(1)));
    let limit = add(Expression::Literal(Literal::U32(MAX_ITERATIONS)));
    // Computed expressions must be "emitted" before use; the ones above
    // don't need to be.
    let count = add(Expression::Load { pointer });
    let next = add(Expression::Binary {
        op: BinaryOperator::Add,
        left: count,
        right: one,
    });
    let over = add(Expression::Binary {
        op: BinaryOperator::Greater,
        left: next,
        right: limit,
    });

    let mut guard = Block::with_capacity(3);
    guard.push(
        Statement::Emit(Range::new_from_bounds(count, over)),
        Span::UNDEFINED,
    );
    guard.push(
        Statement::Store {
            pointer,
            value: next,
        },
        Span::UNDEFINED,
    );
    guard.push(
        Statement::If {
            condition: over,
            accept: Block::from_vec(vec![Statement::Break]),
            reject: Block::new(),
        },
        Span::UNDEFINED,
    );
    guard
}
