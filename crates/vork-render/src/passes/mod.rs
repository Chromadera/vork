//! One function per GPU pass. frame.rs calls them in order; each records into the
//! frame's command encoder from the installed scene and the frame slot.
pub(crate) mod deform;
pub(crate) mod depth;
pub(crate) mod camera_cull;
pub(crate) mod forward;
pub(crate) mod subsurface;
pub(crate) mod resolve;
pub(crate) mod transparent;

pub(crate) mod reflections;

pub(crate) mod ao;

pub(crate) mod path;
pub(crate) mod clusters;
pub(crate) mod decoupled;
