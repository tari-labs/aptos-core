// Copyright (c) Aptos Foundation
// SPDX-License-Identifier: Apache-2.0

//! This module implements a checker that looks for sequences (of len > 1)
//! within binary operations. The v1 compiler's evaluation order semantics
//! in the presence of sequences within binary operations is not easily
//! understood or explainable (see example below).
//!
//! Therefore, in compiler v2 (and above), if the language version is less than
//! 2.0, we will emit an error in such cases. We expect such uses to be rare,
//! and the user can easily rewrite the code to get explicit evaluation order
//! that they want.
//!
//! In language version 2.0 and above, we will allow sequences within binary
//! operations, but the evaluation order will be consistently left-to-right,
//! following the evaluation order semantics used in normal function calls.
//!
//! Consider the following examples to see some samples of evaluation order used
//! by compiler v1 in the presence of sequences within the context of binary
//! operations. They are meant to showcase how concisely describing the v1 ordering
//! is hard (as opposed to, a left-to-right evaluation ordering everywhere).
//!
//! We number the sub-expressions in their order of their evaluation.
//! Some (sub-)expressions are left un-numbered if they are irrelevant to the
//! understanding of the evaluation order.
//!
//! case 1: `add` is a user-defined function.
//! ```move
//! let x = 1;
//! add({x = x - 1; x + 8}, {x = x + 3; x - 3}) + {x = x * 2; x * 2}
//!      ^^^^^^^^^  ^^^^^    ^^^^^^^^^  ^^^^^      ^^^^^^^^^  ^^^^^
//!         |        |         |          |            |        |
//!         |        |         |          |            |        |
//!         1        |         |          |            |        |
//!                  2         |          |            |        |
//!                            3          |            |        |
//!                                       |            4        |
//!                                       5                     |
//!                                                             6
//! ```
//!
//! case 2:
//! ```move
//! fun aborter(x: u64): u64 {
//!     abort x
//! }
//!
//! public fun test(): u64 {
//!     let x = 1;
//!     aborter(x) + {x = x + 1; aborter(x + 100); x} + x
//!     ^^^^^^^^^^    ^^^^^^^^^  ^^^^^^^^^^^^^^^^
//!        |              |              |
//!        |              1              |
//!        |                             2
//!     never evaluated
//! }
//! ```
//!
//! case 3:
//! ```move
//! (abort 0) + {(abort 14); 0} + 0
//!  ^^^^^^^      ^^^^^^^^
//!     |              |
//!     1              |
//!                 never evaluated
//! ```
//!
//! case 4:
//! ```move
//! {250u8 + 50u8} + {abort 55; 5u8}
//!  ^^^^^^^^^^^^     ^^^^^^^^
//!      |               |
//!      |               1
//!   never evaluated
//! ```
//!
//! case 5:
//! ```move
//! let x = 1;
//! x + {x = x + 1; x} + {x = x + 1; x}
//! ^    ^^^^^^^^^  ^     ^^^^^^^^^  ^
//! |       |       |        |       |
//! |       1       |        |       |
//! |               |        2       |
//! 3               3                3
//! ```

use codespan_reporting::diagnostic::Severity;
use move_model::{
    ast::ExpData,
    model::{FunctionEnv, GlobalEnv},
};
use std::collections::BTreeMap;

/// Perform the check detailed in the module documentation at the top of this file.
/// This check is performed on all non-native functions in all target modules.
/// Violations of the check are reported as errors on the `env`.
pub fn checker(env: &mut GlobalEnv) {
    for module in env.get_modules() {
        if module.is_target() {
            for function in module.get_functions() {
                if function.is_native() {
                    continue;
                }
                check_function(&function);
            }
        }
    }
}

/// Perform the check detailed in the module documentation on the code in `function`.
/// Violations of the check are reported as errors on the `GlobalEnv` of the `function`.
fn check_function(function: &FunctionEnv) {
    if let Some(def) = function.get_def() {
        // Maintain a stack of pairs (binary operation's node id, binary operation),
        // as we descend and ascend the AST.
        let mut binop_stack = Vec::new();
        // Maintain a mapping from the binary operation's node id to the first sequence
        // node id within the binary operation (as well as the binary operation itself).
        // We pick the first arbitrarily, instead of reporting all of them.
        // We use this mapping later to report errors.
        let mut errors = BTreeMap::new();
        let mut visitor = |post: bool, e: &ExpData| {
            use ExpData::*;
            match e {
                Call(id, op, _) if op.is_binop() => {
                    if !post {
                        binop_stack.push((*id, op.clone()));
                    } else {
                        binop_stack.pop().expect("unbalanced");
                    }
                },
                Sequence(id, seq) if seq.len() > 1 => {
                    // Likely better UX to use the top-most binary operation to report the error.
                    if let Some((binop_id, binop)) = binop_stack.first() {
                        // Note: if this check is too restrictive, we can relax it to allow
                        // certain cases, such as:
                        // - sequence is made of pure expressions (thus, eval order doesn't matter)
                        // - sequences within binops are guaranteed to be mutually non-conflicting
                        errors.entry(*binop_id).or_insert((*id, binop.clone()));
                    }
                },
                _ => {},
            }
            true
        };
        def.visit_pre_post(&mut visitor);
        let env = function.module_env.env;
        for (binop_id, (seq_id, binop)) in errors {
            let binop_loc = env.get_node_loc(binop_id);
            let seq_loc = env.get_node_loc(seq_id);
            let labels = vec![(seq_loc, "non-empty sequence".to_owned())];
            let binop_as_str = binop.to_string_if_binop().expect("binop");
            let notes = vec![
                "To compile this code, either:".to_owned(),
                "1. upgrade to language version 2.0 or above,".to_owned(),
                "2. rewrite the code to remove sequences from directly within binary operations,"
                    .to_owned(),
                "   e.g., save intermediate results providing explicit order.".to_owned(),
            ];
            env.diag_with_primary_notes_and_labels(
                Severity::Error,
                &binop_loc,
                &format!(
                    "Non-empty sequence within the context of the binary operation `{}`",
                    binop_as_str
                ),
                &format!("binary operation `{}`", binop_as_str),
                notes,
                labels,
            );
        }
    }
}
