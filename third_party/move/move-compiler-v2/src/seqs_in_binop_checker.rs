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
//! operations, but the evaluation order will be left-to-right, following
//! the evaluation order semantics used in function calls.
//!
//! As an example, consider the following move code, where `add` is a
//! user-defined function that takes two arguments and returns their sum:
//! ```move
//! let x = 1;
//! add({x = x - 1; x + 8}, {x = x + 3; x - 3}) + {x = x * 2; x * 2}
//! ```
//! In v1, the code above is translated into the following code, which is not
//! easily explainable or understood:
//! ```ir
//! let x = 1;
//! x = x - 1;
//! let temp1 = x + 8; // needed to get left-to-right evaluation order
//! x = x + 3;
//! x = x * 2; // note that we do not save `x - 3` in a temp,
//!            // so we won't get left-to-right evaluation order
//! add(temp1, x - 3) + x * 2
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
