use crate::maker::Maker;

/// Attach tracing to a [Component]
#[derive(Clone)]
pub struct WithTracing<Component> {
    pub inner: Component,
}

impl<Component> WithTracing<Component> {
    pub fn wrap(backlog: Component) -> Self {
        Self { inner: backlog }
    }
}

/// Any [Component] can be instantiated with tracing.
impl<Key, Component, Context> Maker<Key, Context> for WithTracing<Component>
where
    Component: Maker<Key, Context>,
{
    fn make(key: Key, ctx: &Context) -> Self {
        Self::wrap(Component::make(key, ctx))
    }
}
