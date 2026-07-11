//! I3 P0 stack-diet fallback: the explicit-stack (heap worklist) engine that
//! replaces Rust-call-stack recursion for the one truly unbounded recursive
//! axis in this crate — bean/property/constructor-arg/collection nesting
//! depth (see [`crate::DEPTH_LIMIT`]). Frame-dieting alone (boxing large
//! locals, splitting fat frames into `#[inline(never)]` helpers — see
//! `bean::parse_bean`'s and friends' own doc comments for that pass) halved
//! per-level stack cost but could not reach a 256 KiB thread budget at
//! `DEPTH_LIMIT` (256) levels for every shape (measured via
//! `tests/scratch_stack_probe.rs`: `deep_list`/`deep_inner_bean`/`deep_map`,
//! and a depth-255 *valid* document, all still overflowed a 256 KiB thread
//! in both debug and release). This module is the mandated fallback for
//! those paths: nesting depth now costs one [`Frame`] on this `Vec`'s heap
//! allocation per level, not one native call-stack frame.
//!
//! Only the recursive call boundaries that actually grow with document
//! nesting depth move onto this engine:
//! - `bean::parse_bean`'s "a `<property>`/`<constructor-arg>`'s own
//!   value-shaped child is an inner `<bean>`" edge (previously
//!   `inject_value::parse_inner_bean_boxed` calling back into
//!   `bean::parse_bean`).
//! - `collection::parse_collection_value`'s "a list/set/array item, or a
//!   map entry's key/value, is itself a nested `<bean>` or collection" edge.
//!
//! The one other unbounded recursive axis in this crate —
//! `dispatch::parse_beans_body`'s `<beans profile="...">`-in-`<beans>`
//! nesting (`parse_beans_body` → `dispatch_root_child` →
//! `parse_nested_beans` → `parse_beans_body`) — follows this exact same
//! frame-owns-resumable-state / `step`-drives-scanning / results-flow-back
//! shape, but through its own sibling engine (`dispatch::BeansBodyFrame` +
//! `dispatch::run_beans_body`), not through this module's `Frame`/`run`.
//! The two never need to interoperate (a `<beans>` element is never
//! reachable through a `<bean>`'s own children or a collection's own
//! items/entries, and vice versa), so keeping them separate avoids adding
//! "can never actually happen" cases to either one's `Frame`/`Completed`
//! enum — see `dispatch.rs`'s own matching doc comment (right above
//! `BeansBodyFrame`) for the full rationale.
//!
//! Every other per-level concern (bean attribute parsing,
//! `<qualifier>`/`<meta>`/`<lookup-method>`/`<replaced-method>`/decorator
//! handling, `<props>` entries) is bounded, non-recursive work that was
//! never part of the unbounded recursion — it stays exactly as it was,
//! called directly (not through this engine) from [`crate::bean::BeanFrame`]/
//! [`crate::collection::ListLikeFrame`]/[`crate::collection::MapFrame`]'s own
//! `step` methods.
//!
//! # How it works
//!
//! [`Frame`] is one suspended `parse_bean`/`parse_collection_value` call —
//! everything that call's own (now-unwound) Rust stack frame used to hold
//! across its recursive descent lives here instead. [`run`] drives a
//! `Vec<Frame>` to completion: each iteration either advances the top frame
//! in place (`Advance::Continue` — e.g. it resolved a leaf value or
//! processed a non-recursive bean child), pushes a new frame for a
//! value-shaped child that needs its own recursive resolution
//! (`Advance::Push` — the "descend" step, replacing a Rust call), or
//! finishes the top frame (`Advance::Finished` — the "return" step,
//! replacing a Rust `return`): the finished frame is popped, converted to a
//! [`Completed`] result via [`Frame::finish`], and delivered to whatever
//! frame is now on top (or, once the stack is empty, returned as this whole
//! engine run's own result).
//!
//! A completed inner `<bean>` is always wrapped into `InjectValue::Inner(..)`
//! before being delivered to a non-empty stack ([`wrap_as_value`]) — the
//! same wrapping `inject_value::parse_inner_bean_boxed` used to apply
//! inline before its own recursive call returned. Only the very top-level
//! frame (the one `bean::parse_bean`'s own public entry point pushed) is
//! allowed to surface a bare [`Completed::Bean`] to its caller.

use crate::model::{Bean, Diagnostic, InjectValue};

/// One suspended `parse_bean` (see [`crate::bean::BeanFrame`]) or
/// `parse_collection_value` (see [`crate::collection::ListLikeFrame`]/
/// [`crate::collection::MapFrame`]) call, living on [`run`]'s own `Vec`
/// instead of the real call stack.
pub(crate) enum Frame<'a> {
    Bean(crate::bean::BeanFrame<'a>),
    ListLike(crate::collection::ListLikeFrame<'a>),
    Map(crate::collection::MapFrame<'a>),
}

/// What a fully-finished [`Frame`] hands back to whichever frame is now on
/// top of the stack (or to [`run`]'s own caller, once the stack is empty).
pub(crate) enum Completed {
    Bean(Box<Bean>),
    Value(Box<InjectValue>),
}

/// One [`Frame`]'s own step result — see this module's own doc comment for
/// the "advance in place / descend / return" framing each variant plays.
pub(crate) enum Advance<'a> {
    /// Descend: push `Frame` and re-enter the driver loop with it on top —
    /// the frame underneath (which requested this) is left exactly as it
    /// was; it only resumes once the pushed frame eventually finishes (see
    /// [`Frame::deliver`]). Boxed — `Frame` itself (a `BeanFrame`/
    /// `ListLikeFrame`/`MapFrame`) is large, and this variant is nested one
    /// level inside `ValueStep::Deferred` at every push site
    /// (`inject_value::begin_resolve_value`/`begin_resolve_collection`), so
    /// boxing here keeps both enums small regardless.
    Push(Box<Frame<'a>>),
    /// Made progress without changing the stack's shape (resolved a leaf
    /// value immediately, processed a non-recursive child, folded a
    /// delivered result into this frame's own accumulator, ...) — call
    /// `step`/`deliver` again on the same (still-top) frame.
    Continue,
    /// Return: this frame has nothing left to do. The driver pops it,
    /// converts it via [`Frame::finish`], and delivers the result to
    /// whatever is now on top (or returns it, if the stack is now empty).
    Finished,
}

impl<'a> Frame<'a> {
    fn step(&mut self, diagnostics: &mut Vec<Diagnostic>) -> Advance<'a> {
        match self {
            Frame::Bean(b) => b.step(diagnostics),
            Frame::ListLike(l) => l.step(diagnostics),
            Frame::Map(m) => m.step(diagnostics),
        }
    }

    fn deliver(
        &mut self,
        value: Box<InjectValue>,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Advance<'a> {
        match self {
            Frame::Bean(b) => b.deliver(value, diagnostics),
            Frame::ListLike(l) => l.deliver(value),
            Frame::Map(m) => m.deliver(value, diagnostics),
        }
    }

    /// Consumes a finished frame into its own [`Completed`] result — only
    /// ever called once `step`/`deliver` has returned `Advance::Finished`
    /// for it.
    fn finish(self) -> Completed {
        match self {
            Frame::Bean(b) => Completed::Bean(b.finish()),
            Frame::ListLike(l) => Completed::Value(l.finish()),
            Frame::Map(m) => Completed::Value(m.finish()),
        }
    }
}

/// A completed inner `<bean>` becomes `InjectValue::Inner(..)` before it's
/// delivered to a non-empty stack — see this module's own doc comment.
fn wrap_as_value(completed: Completed) -> Box<InjectValue> {
    match completed {
        Completed::Bean(bean) => crate::inject_value::box_inner_inject_value(bean),
        Completed::Value(value) => value,
    }
}

/// Drives `stack` to completion — see this module's own doc comment for the
/// full step/descend/return framing. `stack` must start with exactly one
/// frame (the call this whole engine run is standing in for); every further
/// frame is pushed/popped internally as nested value resolution demands.
pub(crate) fn run(mut stack: Vec<Frame<'_>>, diagnostics: &mut Vec<Diagnostic>) -> Completed {
    debug_assert!(!stack.is_empty(), "engine started with no initial frame");
    let mut incoming: Option<Box<InjectValue>> = None;
    loop {
        let top = stack
            .last_mut()
            .expect("engine stack emptied while still running");
        let advance = match incoming.take() {
            Some(value) => top.deliver(value, diagnostics),
            None => top.step(diagnostics),
        };
        match advance {
            Advance::Push(frame) => stack.push(*frame),
            Advance::Continue => {}
            Advance::Finished => {
                let frame = stack.pop().expect("the frame that just advanced");
                let completed = frame.finish();
                if stack.is_empty() {
                    return completed;
                }
                incoming = Some(wrap_as_value(completed));
            }
        }
    }
}
