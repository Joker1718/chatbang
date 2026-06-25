//! `declare_tool!` — declarative macro for defining tools with minimal
//! boilerplate (task CB-009).
//!
//! A single invocation generates:
//!  - the per-tool `Args` struct (with `Deserialize` + `JsonSchema` derives),
//!  - the tool struct itself,
//!  - the full `Tool` impl.
//!
//! Both sync and async execute bodies are supported. To register, the caller
//! writes `reg.register(MyTool)` — generating a `register_my_tool` function
//! would require identifier concatenation, which means pulling in the `paste`
//! crate. We chose not to add that dependency; one extra line at the call
//! site is much cheaper than a build-time dependency.

/// Declare a tool with a sync (non-async) execute body.
///
/// ```ignore
/// declare_tool! {
///     tool: EchoTool,
///     args: EchoArgs,
///     name: "echo",
///     description: "echoes back the message",
///     fields: { message: String }
///     execute: |args| {
///         let a: EchoArgs = serde_json::from_value(args.clone())?;
///         Ok(a.message)
///     }
/// }
/// ```
#[macro_export]
macro_rules! declare_tool {
    (
        tool: $tool_ident:ident,
        args: $args_ident:ident,
        name: $name:expr,
        description: $desc:expr,
        fields: { $($field:ident : $ty:ty),* $(,)? }
        execute: |$args_var:ident| $body:expr
    ) => {
        #[derive(::serde::Deserialize, ::schemars::JsonSchema)]
        pub struct $args_ident {
            $(pub $field: $ty),*
        }

        pub struct $tool_ident;

        #[::async_trait::async_trait]
        impl $crate::tools::traits::Tool for $tool_ident {
            fn name(&self) -> &str { $name }
            fn description(&self) -> &str { $desc }
            fn parameters_schema(&self) -> ::schemars::schema::RootSchema {
                ::schemars::schema_for!($args_ident)
            }
            async fn execute(
                &self,
                $args_var: &::serde_json::Value,
            ) -> $crate::tools::error::ToolResult<String> {
                $body
            }
        }
    };
}

/// Declare a tool with an async execute body.
///
/// ```ignore
/// declare_async_tool! {
///     tool: RunCommandTool,
///     args: RunCommandArgs,
///     name: "run_command",
///     description: "run a shell command",
///     fields: { command: String }
///     execute: |args| async move {
///         // ...
///     }
/// }
/// ```
#[macro_export]
macro_rules! declare_async_tool {
    (
        tool: $tool_ident:ident,
        args: $args_ident:ident,
        name: $name:expr,
        description: $desc:expr,
        fields: { $($field:ident : $ty:ty),* $(,)? }
        execute: |$args_var:ident| async move $body:block
    ) => {
        #[derive(::serde::Deserialize, ::schemars::JsonSchema)]
        pub struct $args_ident {
            $(pub $field: $ty),*
        }

        pub struct $tool_ident;

        #[::async_trait::async_trait]
        impl $crate::tools::traits::Tool for $tool_ident {
            fn name(&self) -> &str { $name }
            fn description(&self) -> &str { $desc }
            fn parameters_schema(&self) -> ::schemars::schema::RootSchema {
                ::schemars::schema_for!($args_ident)
            }
            async fn execute(
                &self,
                $args_var: &::serde_json::Value,
            ) -> $crate::tools::error::ToolResult<String> {
                $body
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::tools::registry::ToolRegistry;
    use crate::tools::traits::Tool;

    declare_tool! {
        tool: MacroEchoTool,
        args: MacroEchoArgs,
        name: "macro_echo",
        description: "echoes back the message (defined via declare_tool!)",
        fields: { message: String }
        execute: |args| {
            let a: MacroEchoArgs = serde_json::from_value(args.clone())?;
            Ok(a.message)
        }
    }

    declare_async_tool! {
        tool: MacroAsyncTool,
        args: MacroAsyncArgs,
        name: "macro_async",
        description: "async echo",
        fields: { message: String }
        execute: |args| async move {
            let a: MacroAsyncArgs = serde_json::from_value(args.clone())?;
            Ok(a.message)
        }
    }

    #[tokio::test]
    async fn macro_generates_working_tool() {
        let reg = ToolRegistry::new();
        reg.register(MacroEchoTool);
        let out = reg
            .dispatch("macro_echo", &serde_json::json!({ "message": "hi" }))
            .await
            .unwrap();
        assert_eq!(out, "hi");
    }

    #[tokio::test]
    async fn macro_async_works() {
        let reg = ToolRegistry::new();
        reg.register(MacroAsyncTool);
        let out = reg
            .dispatch("macro_async", &serde_json::json!({ "message": "yo" }))
            .await
            .unwrap();
        assert_eq!(out, "yo");
    }

    #[tokio::test]
    async fn macro_generates_schema() {
        let t = MacroEchoTool;
        let schema = t.parameters_schema();
        assert!(schema
            .schema
            .object
            .as_ref()
            .unwrap()
            .required
            .contains("message"));
    }
}
