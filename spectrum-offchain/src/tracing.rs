use crate::maker::Maker;

/// Attach tracing to a [Component]
#[derive(Clone)]
pub struct Tracing<Component> {
    pub component: Component,
}

impl<Component> Tracing<Component> {
    pub fn attach(component: Component) -> Self {
        Self { component }
    }
}

/// Any [Component] can be instantiated with tracing.
impl<Key, Component, Context> Maker<Key, Context> for Tracing<Component>
where
    Component: Maker<Key, Context>,
{
    fn make(key: Key, ctx: &Context) -> Self {
        Self::attach(Component::make(key, ctx))
    }
}
