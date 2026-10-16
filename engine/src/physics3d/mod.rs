mod body;
mod solve;
mod world;

pub use body::{RigidBody, box_inertia};
pub use solve::{ContactParams, solve_contacts};
pub use world::{Contact, Hit, StaticWorld, Triangle};
