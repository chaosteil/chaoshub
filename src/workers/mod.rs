//! Background tasks. They call the adapters and send `bus::Msg`.

mod ci;
mod detail;
mod jj;
mod prefetch;
mod startup;
mod supervisor;
mod sync;
