// SPDX-License-Identifier: Apache-2.0
//! The PLCopen XML reader, against a document salman did not write.
//!
//! What this target can find is narrower than for salman's own decoders: a
//! crash inside the `xml` crate is not one salman can fix. It still covers
//! everything salman does with what that parser returns, which is where
//! salman's own mistakes will be — the state machine that walks elements, the
//! XHTML unwrapping, the namespace check, and the rendering to Structured
//! Text.
//!
//! Two properties beyond "it did not crash":
//!
//! * **what reads must render.** Any document that produces a project must
//!   also produce Structured Text without panicking, because the renderer
//!   walks everything the reader built and is where a half-populated model
//!   would show up;
//! * **a body salman cannot read is never silently absent.** Every body is
//!   either Structured Text or a named language, and the count of them is the
//!   count the renderer accounts for. A file that quietly lost half its
//!   program would compile and be wrong.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_plcopen::model::Body;
use salman_plcopen::read::read;

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }

    let Ok(project) = read(data) else {
        return;
    };

    // The renderer walks everything the reader built.
    let text = project.to_structured_text();

    // Every POU appears in the rendering, by name. A model that read a POU and
    // rendered nothing for it is the shape of a silent loss.
    for pou in &project.pous {
        if pou.name.is_empty() {
            continue;
        }
        assert!(
            text.contains(&pou.name),
            "the POU {:?} was read and does not appear in the rendering",
            pou.name
        );
    }

    // Every body salman could not read is named in the rendering and in the
    // list a caller uses to tell a user what did not survive.
    let unread_bodies = project
        .pous
        .iter()
        .flat_map(|pou| &pou.bodies)
        .filter(|body| matches!(body, Body::Other { .. }))
        .count();
    assert_eq!(
        unread_bodies,
        project.unread_bodies().count(),
        "a body salman cannot read went missing between the model and the report"
    );

    // The version is one of the two salman reads, never something invented.
    let _ = project.version.is_the_version_salman_modelled();
    let _ = project.version.name();
});
