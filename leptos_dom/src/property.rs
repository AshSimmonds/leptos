use leptos_reactive::Scope;
use wasm_bindgen::JsValue;

/// Represents the different possible values an element property could have,
/// allowing you to do fine-grained updates to single fields.
///
/// This mostly exists for the [`view`](https://docs.rs/leptos_macro/latest/leptos_macro/macro.view.html)
/// macro’s use. You usually won't need to interact with it directly.
pub enum Property {
    /// A static JavaScript value.
    Value(JsValue),
    /// A (presumably reactive) function, which will be run inside an effect to toggle the class.
    Fn(Box<dyn Fn() -> JsValue>),
}

/// Converts some type into a [Property].
///
/// This is implemented by default for Rust primitive types, [String] and friends, and [JsValue].
pub trait IntoProperty {
    /// Converts the object into a [Property].
    fn into_property(self, cx: Scope) -> Property;
}

impl<T, U> IntoProperty for T
where
    T: Fn() -> U + 'static,
    U: Into<JsValue>,
{
    fn into_property(self, _cx: Scope) -> Property {
        let modified_fn = Box::new(move || self().into());
        Property::Fn(modified_fn)
    }
}

macro_rules! prop_type {
    ($prop_type:ty) => {
        impl IntoProperty for $prop_type {
            fn into_property(self, _cx: Scope) -> Property {
                Property::Value(self.into())
            }
        }

        impl IntoProperty for Option<$prop_type> {
            fn into_property(self, _cx: Scope) -> Property {
                Property::Value(self.into())
            }
        }
    };
}

prop_type!(JsValue);
prop_type!(String);
prop_type!(&String);
prop_type!(&str);
prop_type!(usize);
prop_type!(u8);
prop_type!(u16);
prop_type!(u32);
prop_type!(u64);
prop_type!(u128);
prop_type!(isize);
prop_type!(i8);
prop_type!(i16);
prop_type!(i32);
prop_type!(i64);
prop_type!(i128);
prop_type!(f32);
prop_type!(f64);
prop_type!(bool);
