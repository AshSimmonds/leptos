use cfg_if::cfg_if;

use crate::runtime::{with_runtime, RuntimeId};
use crate::{hydration::SharedContext, EffectId, ResourceId, SignalId};
use crate::{PinnedFuture, SuspenseContext};
use futures::stream::FuturesUnordered;
use std::collections::HashMap;
use std::fmt::Debug;
use std::{future::Future, pin::Pin};

#[doc(hidden)]
#[must_use = "Scope will leak memory if the disposer function is never called"]
/// Creates a new reactive system and root reactive scope and runs the function within it.
///
/// This should usually only be used once, at the root of an application, because its reactive
/// values will not have access to values created under another `create_scope`.
///
/// You usually don't need to call this manually.
pub fn create_scope(runtime: RuntimeId, f: impl FnOnce(Scope) + 'static) -> ScopeDisposer {
    runtime.run_scope_undisposed(f, None).2
}

#[doc(hidden)]
#[must_use = "Scope will leak memory if the disposer function is never called"]
/// Creates a new reactive system and root reactive scope, and returns them.
///
/// This should usually only be used once, at the root of an application, because its reactive
/// values will not have access to values created under another `create_scope`.
///
/// You usually don't need to call this manually.
pub fn raw_scope_and_disposer(runtime: RuntimeId) -> (Scope, ScopeDisposer) {
    runtime.raw_scope_and_disposer()
}

#[doc(hidden)]
/// Creates a temporary scope, runs the given function, disposes of the scope,
/// and returns the value returned from the function. This is very useful for short-lived
/// applications like SSR, where actual reactivity is not required beyond the end
/// of the synchronous operation.
///
/// You usually don't need to call this manually.
pub fn run_scope<T>(runtime: RuntimeId, f: impl FnOnce(Scope) -> T + 'static) -> T {
    runtime.run_scope(f, None)
}

#[doc(hidden)]
#[must_use = "Scope will leak memory if the disposer function is never called"]
/// Creates a temporary scope and run the given function without disposing of the scope.
/// If you do not dispose of the scope on your own, memory will leak.
///
/// You usually don't need to call this manually.
pub fn run_scope_undisposed<T>(
    runtime: RuntimeId,
    f: impl FnOnce(Scope) -> T + 'static,
) -> (T, ScopeId, ScopeDisposer) {
    runtime.run_scope_undisposed(f, None)
}

/// A Each scope can have
/// child scopes, and may in turn have a parent.
///
/// Scopes manage memory within the reactive system. When a scope is disposed, its
/// cleanup functions run and the signals, effects, memos, resources, and contexts
/// associated with it no longer exist and should no longer be accessed.
///
/// You generally won’t need to create your own scopes when writing application code.
/// However, they’re very useful for managing control flow within an application or library.
/// For example, if you are writing a keyed list component, you will want to create a child scope
/// for each row in the list so that you can dispose of its associated signals, etc.
/// when it is removed from the list.
///
/// Every other function in this crate takes a `Scope` as its first argument. Since `Scope`
/// is [Copy] and `'static` this does not add much overhead or lifetime complexity.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub(crate) runtime: RuntimeId,
    pub(crate) id: ScopeId,
}

impl Scope {
    /// The unique identifier for this scope.
    pub fn id(&self) -> ScopeId {
        self.id
    }

    /// Creates a child scope and runs the given function within it, returning a handle to dispose of it.
    ///
    /// The child scope has its own lifetime and disposer, but will be disposed when the parent is
    /// disposed, if it has not been already.
    ///
    /// This is useful for applications like a list or a router, which may want to create child scopes and
    /// dispose of them when they are no longer needed (e.g., a list item has been destroyed or the user
    /// has navigated away from the route.)
    pub fn child_scope(self, f: impl FnOnce(Scope)) -> ScopeDisposer {
        let (_, disposer) = self.run_child_scope(f);
        disposer
    }

    /// Creates a child scope and runs the given function within it, returning the function's return
    /// type and a handle to dispose of it.
    ///
    /// The child scope has its own lifetime and disposer, but will be disposed when the parent is
    /// disposed, if it has not been already.
    ///
    /// This is useful for applications like a list or a router, which may want to create child scopes and
    /// dispose of them when they are no longer needed (e.g., a list item has been destroyed or the user
    /// has navigated away from the route.)
    pub fn run_child_scope<T>(self, f: impl FnOnce(Scope) -> T) -> (T, ScopeDisposer) {
        let (res, child_id, disposer) = self.runtime.run_scope_undisposed(f, Some(self));
        with_runtime(self.runtime, |runtime| {
            let mut children = runtime.scope_children.borrow_mut();
            children
                .entry(self.id)
                .expect("trying to add a child to a Scope that has already been disposed")
                .or_default()
                .push(child_id);
        });
        (res, disposer)
    }

    /// Suspends reactive tracking while running the given function.
    ///
    /// This can be used to isolate parts of the reactive graph from one another.
    ///
    /// ```
    /// # use leptos_reactive::*;
    /// # run_scope(create_runtime(), |cx| {
    /// let (a, set_a) = create_signal(cx, 0);
    /// let (b, set_b) = create_signal(cx, 0);
    /// let c = create_memo(cx, move |_| {
    ///     // this memo will *only* update when `a` changes
    ///     a() + cx.untrack(move || b())
    /// });
    ///
    /// assert_eq!(c(), 0);
    /// set_a(1);
    /// assert_eq!(c(), 1);
    /// set_b(1);
    /// // hasn't updated, because we untracked before reading b
    /// assert_eq!(c(), 1);
    /// set_a(2);
    /// assert_eq!(c(), 3);
    ///
    /// # });
    /// ```
    pub fn untrack<T>(&self, f: impl FnOnce() -> T) -> T {
        with_runtime(self.runtime, |runtime| {
            let prev_observer = runtime.observer.take();
            let untracked_result = f();
            runtime.observer.set(prev_observer);
            untracked_result
        })
    }
}

// Internals

impl Scope {
    pub(crate) fn dispose(self) {
        with_runtime(self.runtime, |runtime| {
            // dispose of all child scopes
            let children = {
                let mut children = runtime.scope_children.borrow_mut();
                children.remove(self.id)
            };

            if let Some(children) = children {
                for id in children {
                    Scope {
                        runtime: self.runtime,
                        id,
                    }
                    .dispose();
                }
            }

            // run cleanups
            if let Some(cleanups) = runtime.scope_cleanups.borrow_mut().remove(self.id) {
                for cleanup in cleanups {
                    cleanup();
                }
            }

            // remove everything we own and run cleanups
            let owned = {
                let owned = runtime.scopes.borrow_mut().remove(self.id);
                owned.map(|owned| owned.take())
            };
            if let Some(owned) = owned {
                for property in owned {
                    match property {
                        ScopeProperty::Signal(id) => {
                            // remove the signal
                            runtime.signals.borrow_mut().remove(id);
                            let subs = runtime.signal_subscribers.borrow_mut().remove(id);

                            // each of the subs needs to remove the signal from its dependencies
                            // so that it doesn't try to read the (now disposed) signal
                            if let Some(subs) = subs {
                                let source_map = runtime.effect_sources.borrow();
                                for effect in subs.borrow().iter() {
                                    if let Some(effect_sources) = source_map.get(*effect) {
                                        effect_sources.borrow_mut().remove(&id);
                                    }
                                }
                            }
                        }
                        ScopeProperty::Effect(id) => {
                            runtime.effects.borrow_mut().remove(id);
                            runtime.effect_sources.borrow_mut().remove(id);
                        }
                        ScopeProperty::Resource(id) => {
                            runtime.resources.borrow_mut().remove(id);
                        }
                    }
                }
            }
        })
    }

    pub(crate) fn with_scope_property(&self, f: impl FnOnce(&mut Vec<ScopeProperty>)) {
        with_runtime(self.runtime, |runtime| {
            let scopes = runtime.scopes.borrow();
            let scope = scopes
                .get(self.id)
                .expect("tried to add property to a scope that has been disposed");
            f(&mut scope.borrow_mut());
        })
    }
}

/// Creates a cleanup function, which will be run when a [Scope] is disposed.
///
/// It runs after child scopes have been disposed, but before signals, effects, and resources
/// are invalidated.
pub fn on_cleanup(cx: Scope, cleanup_fn: impl FnOnce() + 'static) {
    with_runtime(cx.runtime, |runtime| {
        let mut cleanups = runtime.scope_cleanups.borrow_mut();
        let cleanups = cleanups
            .entry(cx.id)
            .expect("trying to clean up a Scope that has already been disposed")
            .or_insert_with(Default::default);
        cleanups.push(Box::new(cleanup_fn));
    })
}

slotmap::new_key_type! {
    /// Unique ID assigned to a [Scope](crate::Scope).
    pub struct ScopeId;
}

#[derive(Debug)]
pub(crate) enum ScopeProperty {
    Signal(SignalId),
    Effect(EffectId),
    Resource(ResourceId),
}

/// Creating a [Scope](crate::Scope) gives you a disposer, which can be called
/// to dispose of that reactive scope.
///
/// This will
/// 1. dispose of all child `Scope`s
/// 2. run all cleanup functions defined for this scope by [on_cleanup](crate::on_cleanup).
/// 3. dispose of all signals, effects, and resources owned by this `Scope`.
pub struct ScopeDisposer(pub(crate) Box<dyn FnOnce()>);

impl ScopeDisposer {
    /// Disposes of a reactive [Scope](crate::Scope).
    ///
    /// This will
    /// 1. dispose of all child `Scope`s
    /// 2. run all cleanup functions defined for this scope by [on_cleanup](crate::on_cleanup).
    /// 3. dispose of all signals, effects, and resources owned by this `Scope`.
    pub fn dispose(self) {
        (self.0)()
    }
}

impl Scope {
    // hydration-specific code
    cfg_if! {
        if #[cfg(any(feature = "hydrate", doc))] {
            /// `hydrate` only: Whether we're currently hydrating the page.
            pub fn is_hydrating(&self) -> bool {
                with_runtime(self.runtime, |runtime| {
                runtime.shared_context.borrow().is_some()
                })
            }

            /// `hydrate` only: Begins the hydration process.
            pub fn start_hydration(&self, element: &web_sys::Element) {
                with_runtime(self.runtime, |runtime| {
                    runtime.start_hydration(element);
                })
            }

            /// `hydrate` only: Ends the hydration process.
            pub fn end_hydration(&self) {
                with_runtime(self.runtime, |runtime| {
                    runtime.end_hydration();
                })
            }

            /// `hydrate` only: Gets the next element in the hydration queue, either from the
            /// server-rendered DOM or from the template.
            pub fn get_next_element(&self, template: &web_sys::Element) -> web_sys::Element {
                use wasm_bindgen::{JsCast, UnwrapThrowExt};

                let cloned_template = |t: &web_sys::Element| {
                    let t = t
                        .unchecked_ref::<web_sys::HtmlTemplateElement>()
                        .content()
                        .clone_node_with_deep(true)
                        .expect_throw("(get_next_element) could not clone template")
                        .unchecked_into::<web_sys::Element>()
                        .first_element_child()
                        .expect_throw("(get_next_element) could not get first child of template");
                    t
                };

                with_runtime(self.runtime, |runtime| {
                    if let Some(ref mut shared_context) = &mut *runtime.shared_context.borrow_mut() {
                        if shared_context.context.is_some() {
                            let key = shared_context.next_hydration_key();
                            let node = shared_context.registry.remove(&key);

                            //log::debug!("(hy) searching for {key}");

                            if let Some(node) = node {
                                //log::debug!("(hy) found {key}");
                                shared_context.completed.push(node.clone());
                                node
                            } else {
                                //log::debug!("(hy) did NOT find {key}");
                                cloned_template(template)
                            }
                        } else {
                            cloned_template(template)
                        }
                    } else {
                        cloned_template(template)
                    }
                })
            }
        }
    }

    /// `hydrate` only: Given the current node, gets the span of the next component that has
    /// been marked for hydration, returning its starting node and the set of all its nodes.
    #[cfg(any(feature = "csr", feature = "hydrate", doc))]
    pub fn get_next_marker(&self, start: &web_sys::Node) -> (web_sys::Node, Vec<web_sys::Node>) {
        let mut end = Some(start.clone());
        let mut count = 0;
        let mut current = Vec::new();
        let mut start = start.clone();

        with_runtime(self.runtime, |runtime| {
            if runtime
                .shared_context
                .borrow()
                .as_ref()
                .map(|sc| sc.context.as_ref())
                .is_some()
            {
                while let Some(curr) = end {
                    start = curr.clone();
                    if curr.node_type() == 8 {
                        // COMMENT
                        let v = curr.node_value();
                        if v == Some("#".to_string()) {
                            count += 1;
                        } else if v == Some("/".to_string()) {
                            count -= 1;
                            if count == 0 {
                                current.push(curr.clone());
                                return (curr, current);
                            }
                        }
                    }
                    current.push(curr.clone());
                    end = curr.next_sibling();
                }
            }

            (start, current)
        })
    }

    /// On either the server side or the browser side, generates the next key in the hydration process.
    pub fn next_hydration_key(&self) -> String {
        with_runtime(self.runtime, |runtime| {
            let mut sc = runtime.shared_context.borrow_mut();
            if let Some(ref mut sc) = *sc {
                sc.next_hydration_key()
            } else {
                let mut new_sc = SharedContext::default();
                let id = new_sc.next_hydration_key();
                *sc = Some(new_sc);
                id
            }
        })
    }

    /// Runs the given function with the next hydration context.
    pub fn with_next_context<T>(&self, f: impl FnOnce() -> T) -> T {
        with_runtime(self.runtime, |runtime| {
            if runtime
                .shared_context
                .borrow()
                .as_ref()
                .and_then(|sc| sc.context.as_ref())
                .is_some()
            {
                let c = {
                    if let Some(ref mut sc) = *runtime.shared_context.borrow_mut() {
                        if let Some(ref mut context) = sc.context {
                            let next = context.next_hydration_context();
                            Some(std::mem::replace(context, next))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                let res = self.untrack(f);

                if let Some(ref mut sc) = *runtime.shared_context.borrow_mut() {
                    sc.context = c;
                }
                res
            } else {
                self.untrack(f)
            }
        })
    }

    /// Returns IDs for all [Resource](crate::Resource)s found on any scope.
    pub fn all_resources(&self) -> Vec<ResourceId> {
        with_runtime(self.runtime, |runtime| runtime.all_resources())
    }

    /// The current key for an HTML fragment created by server-rendering a `<Suspense/>` component.
    pub fn current_fragment_key(&self) -> String {
        with_runtime(self.runtime, |runtime| {
            runtime
                .shared_context
                .borrow()
                .as_ref()
                .map(|context| context.current_fragment_key())
                .unwrap_or_else(|| String::from("0f"))
        })
    }

    /// Returns IDs for all [Resource](crate::Resource)s found on any scope.
    pub fn serialization_resolvers(&self) -> FuturesUnordered<PinnedFuture<(ResourceId, String)>> {
        with_runtime(self.runtime, |runtime| runtime.serialization_resolvers())
    }

    /// Registers the given [SuspenseContext](crate::SuspenseContext) with the current scope,
    /// calling the `resolver` when its resources are all resolved.
    pub fn register_suspense(
        &self,
        context: SuspenseContext,
        key: &str,
        resolver: impl FnOnce() -> String + 'static,
    ) {
        use crate::create_isomorphic_effect;
        use futures::StreamExt;

        with_runtime(self.runtime, |runtime| {
            if let Some(ref mut shared_context) = *runtime.shared_context.borrow_mut() {
                let (tx, mut rx) = futures::channel::mpsc::unbounded();

                create_isomorphic_effect(*self, move |_| {
                    let pending = context.pending_resources.try_with(|n| *n).unwrap_or(0);
                    if pending == 0 {
                        _ = tx.unbounded_send(());
                    }
                });

                shared_context.pending_fragments.insert(
                    key.to_string(),
                    Box::pin(async move {
                        rx.next().await;
                        resolver()
                    }),
                );
            }
        })
    }

    /// The set of all HTML fragments current pending, by their keys (see [Self::current_fragment_key]).
    pub fn pending_fragments(&self) -> HashMap<String, Pin<Box<dyn Future<Output = String>>>> {
        with_runtime(self.runtime, |runtime| {
            if let Some(ref mut shared_context) = *runtime.shared_context.borrow_mut() {
                std::mem::take(&mut shared_context.pending_fragments)
            } else {
                HashMap::new()
            }
        })
    }
}

impl Debug for ScopeDisposer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ScopeDisposer").finish()
    }
}
