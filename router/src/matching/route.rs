use std::rc::Rc;

use leptos::leptos_dom::Child;
use leptos::*;

#[derive(Clone)]
pub struct RouteDefinition {
    pub path: &'static str,
    pub children: Vec<RouteDefinition>,
    pub element: Rc<dyn Fn(Scope) -> Child>,
}

impl std::fmt::Debug for RouteDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouteDefinition")
            .field("path", &self.path)
            .field("children", &self.children)
            .finish()
    }
}

impl PartialEq for RouteDefinition {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.children == other.children
    }
}

impl Default for RouteDefinition {
    fn default() -> Self {
        Self {
            path: Default::default(),
            children: Default::default(),
            element: Rc::new(|_| Child::Null),
        }
    }
}
