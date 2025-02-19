use leptos_dom::{Child, IntoChild};
use leptos_reactive::{provide_context, Scope, SignalSetter, SuspenseContext};
use typed_builder::TypedBuilder;

/// Props for the [Suspense](crate::Suspense) component, which shows a fallback
/// while [Resource](leptos_reactive::Resource)s are being read.
#[derive(TypedBuilder)]
pub struct TransitionProps<F, E, G>
where
    F: IntoChild + Clone,
    E: IntoChild,
    G: Fn() -> E,
{
    /// Will be displayed while resources are pending.
    pub fallback: F,
    /// A function that will be called when the component transitions into or out of
    /// the `pending` state, with its argument indicating whether it is pending (`true`)
    /// or not pending (`false`).
    #[builder(default, setter(strip_option, into))]
    pub set_pending: Option<SignalSetter<bool>>,
    /// Will be displayed once all resources have resolved.
    pub children: Box<dyn Fn() -> Vec<G>>,
}

/// If any [Resource](leptos_reactive::Resource)s are read in the `children` of this
/// component, it will show the `fallback` while they are loading. Once all are resolved,
/// it will render the `children`. Unlike [`Suspense`](crate::Suspense), this will not fall
/// back to the `fallback` state if there are further changes after the initial load.
///
/// Note that the `children` will be rendered initially (in order to capture the fact that
/// those resources are read under the suspense), so you cannot assume that resources have
/// `Some` value in `children`.
///
/// ```
/// # use leptos_reactive::*;
/// # use leptos_core::*;
/// # use leptos_macro::*;
/// # use leptos_dom::*; use leptos::*;
/// # run_scope(create_runtime(), |cx| {
/// # if cfg!(not(any(feature = "csr", feature = "hydrate", feature = "ssr"))) {
/// async fn fetch_cats(how_many: u32) -> Result<Vec<String>, ()> { Ok(vec![]) }
///
/// let (cat_count, set_cat_count) = create_signal::<u32>(cx, 1);
/// let (pending, set_pending) = create_signal(cx, false);
///
/// let cats = create_resource(cx, cat_count, |count| fetch_cats(count));
///
/// view! { cx,
///   <div>
///     <Transition
///       fallback={"Loading...".to_string()}
///       set_pending=set_pending
///     >
///       {move || {
///           cats.read().map(|data| match data {
///             Err(_) => view! { cx,  <pre>"Error"</pre> },
///             Ok(cats) => view! { cx,
///               <div>{
///                 cats.iter()
///                   .map(|src| {
///                     view! { cx,
///                       <img src={src}/>
///                     }
///                   })
///                   .collect::<Vec<_>>()
///               }</div>
///             },
///           })
///         }
///       }
///     </Transition>
///   </div>
/// };
/// # }
/// # });
/// ```
#[allow(non_snake_case)]
pub fn Transition<F, E, G>(cx: Scope, props: TransitionProps<F, E, G>) -> impl Fn() -> Child
where
    F: IntoChild + Clone,
    E: IntoChild,
    G: Fn() -> E + 'static,
{
    let context = SuspenseContext::new(cx);

    // provide this SuspenseContext to any resources below it
    provide_context(cx, context);

    let child = (props.children)().swap_remove(0);

    render_transition(cx, context, props.fallback, child, props.set_pending)
}

#[cfg(any(feature = "csr", feature = "hydrate"))]
fn render_transition<'a, F, E, G>(
    cx: Scope,
    context: SuspenseContext,
    fallback: F,
    child: G,
    set_pending: Option<SignalSetter<bool>>,
) -> impl Fn() -> Child
where
    F: IntoChild + Clone,
    E: IntoChild,
    G: Fn() -> E,
{
    use std::cell::{Cell, RefCell};

    let has_rendered_once = Cell::new(false);
    let prev_child = RefCell::new(Child::Null);

    move || {
        if context.ready() {
            has_rendered_once.set(true);
            let current_child = (child)().into_child(cx);
            *prev_child.borrow_mut() = current_child.clone();
            if let Some(pending) = &set_pending {
                pending.set(false);
            }
            current_child
        } else if has_rendered_once.get() {
            if let Some(pending) = &set_pending {
                pending.set(true);
            }
            prev_child.borrow().clone()
        } else {
            if let Some(pending) = &set_pending {
                pending.set(true);
            }
            let fallback = fallback.clone().into_child(cx);
            *prev_child.borrow_mut() = fallback.clone();
            fallback
        }
    }
}

#[cfg(not(any(feature = "csr", feature = "hydrate")))]
fn render_transition<'a, F, E, G>(
    cx: Scope,
    context: SuspenseContext,
    fallback: F,
    orig_child: G,
    set_pending: Option<SignalSetter<bool>>,
) -> impl Fn() -> Child
where
    F: IntoChild + Clone,
    E: IntoChild,
    G: Fn() -> E + 'static,
{
    use leptos_dom::IntoAttribute;
    use leptos_macro::view;

    _ = set_pending;

    let initial = {
        // run the child; we'll probably throw this away, but it will register resource reads
        let mut child = orig_child().into_child(cx);
        while let Child::Fn(f) = child {
            child = (f.borrow_mut())();
        }

        // no resources were read under this, so just return the child
        if context.pending_resources.get() == 0 {
            child
        }
        // show the fallback, but also prepare to stream HTML
        else {
            let key = cx.current_fragment_key();
            cx.register_suspense(context, &key, move || {
                orig_child().into_child(cx).as_child_string()
            });

            // return the fallback for now, wrapped in fragment identifer
            Child::Node(view! { cx, <div data-fragment-id=key>{fallback.into_child(cx)}</div> })
        }
    };
    move || initial.clone()
}
