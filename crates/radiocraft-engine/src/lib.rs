//! RadioCraft's engine: floor plans read from DXF drawings and cut from 3D models, the materials
//! of their walls, routers, and the coverage model that predicts the signal across the floor.
//! Everything a user does is a command ([`Engine::execute`]); the window and the CLI share them.

#![forbid(unsafe_code)]

pub mod devices;
pub mod dxf;
pub mod engine;
pub mod materials;
pub mod mesh;
pub mod model;
pub mod picture;
pub mod placement;
pub mod plan;
pub mod radio;
pub mod report;
pub mod survey;
pub mod views;

pub use engine::{COMMANDS, Coverage, Engine, EngineError, Field, Power, Router, Settings};
