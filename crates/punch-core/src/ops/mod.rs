//! Domain operations: the only write path for every adapter.
mod task;
pub(crate) use task::bump_revision;

// Each phase registers an operation module on one line and retains the flat API.
macro_rules! operation {
    ($module:ident) => {
        mod $module;
        pub use $module::*;
    };
}
operation!(project);
operation!(add);
operation!(list);
operation!(show);
operation!(move_task);
operation!(edit);
operation!(note);
operation!(archive);
operation!(ready);
operation!(plan);

operation!(sync);
operation!(claim);
operation!(bc);
operation!(update);
