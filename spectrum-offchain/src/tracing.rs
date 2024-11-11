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
impl<Component, Context> Maker<Context> for WithTracing<Component>
where
    Component: Maker<Context>,
{
    fn make(ctx: &Context) -> Self {
        Self::wrap(Component::make(ctx))
    }
}
