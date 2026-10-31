//! Adapter for the GitHub GraphQL and REST APIs.
#![allow(dead_code, reason = "the workers use the client in a later phase")]

mod budget;
mod error;
mod graphql;
mod parse;
mod queries;
mod rest;
mod token;

/// The GitHub client. A placeholder: phase B replaces it with the real
/// client.
#[derive(Debug)]
pub(crate) struct GithubClient;
