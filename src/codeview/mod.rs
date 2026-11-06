//! codeview: the comment files that chaoshub reads.
//!
//! codeview keeps the review comments in one markdown file per review.
//! `store` reads them. chaoshub never writes these files. To send
//! comments, the `codeview_send` action runs codeview in a headless nvim,
//! and codeview marks the comments as sent. The comments are plain data
//! in `domain::codeview`.

pub(crate) mod store;
