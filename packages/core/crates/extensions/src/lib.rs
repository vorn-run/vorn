//! Extensions: the one kind of package that extends Vorn, and the bridge
//! that runs them.
//!
//! A package is a folder with a manifest ([`manifest`]), installed from git
//! or linked in place ([`install`], [`pack`]). Its program speaks the
//! extension API, defined in `wit/extension.wit` and generated into [`api`],
//! or the connector protocol of 0.8 ([`connector`], [`host`]). Every program
//! runs under one [`lifecycle`]: started when first needed, stopped when
//! idle, and held to a cap. The [`router`] carries calls and events to them
//! and answers what they ask through the [`gate`].
//!
//! Nothing here knows how sessions are kept or how a client is reached; the
//! daemon that hosts extensions hands those in.

pub mod activation;
pub mod api;
pub mod bridge;
pub mod child;
pub mod connector;
pub mod events;
pub mod footer;
pub mod gate;
pub mod grants;
pub mod host;
pub mod install;
pub mod js;
pub mod lifecycle;
pub mod links;
pub mod manifest;
pub mod netproxy;
pub mod pack;
pub mod page;
pub mod polls;
pub mod router;
pub mod sandbox;
pub mod signin;
pub mod token;
pub mod usage;
pub mod views;
