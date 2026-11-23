//! Picking: what was drawn under a pixel in the last frame (Options::pick).

use crate::math::Vec3;

/// `Pick::kind`: nothing.
pub const PICK_NONE: i32 = 0;
/// `Pick::kind`: a grid face.
pub const PICK_GRID: i32 = 1;
/// `Pick::kind`: a sprite.
pub const PICK_SPRITE: i32 = 2;
/// `Pick::kind`: a triangle list.
pub const PICK_MESH: i32 = 3;

/// What one pick id stands for (recorded per draw call, per grid face).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickEntry {
    /// What was drawn (PICK_*).
    pub kind: i32,
    /// Tag of the sprite or mesh.
    pub tag: u32,
    /// Grid cell x (-1 for others).
    pub cell_x: i32,
    /// Grid cell z (-1 for others).
    pub cell_z: i32,
    /// Grid storey (-1 for others).
    pub level: i32,
    /// Grid side (-1 for others).
    pub side: i32,
    /// Grid wall part (PART_*).
    pub part: i32,
}

/// Result of `Renderer::pick`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    /// PICK_GRID, PICK_SPRITE or PICK_MESH.
    pub kind: i32,
    /// Tag of the sprite or mesh; 0 for grid faces.
    pub tag: u32,
    /// Grid faces only (else -1): cell and side (NORTH..WEST, SIDE_FLOOR, SIDE_CEILING, SIDE_STEP).
    pub cell_x: i32,
    /// Grid cell z (-1 for others).
    pub cell_z: i32,
    /// Grid faces only (else -1): storey (level) of the cell.
    pub level: i32,
    /// Grid side (-1 for others).
    pub side: i32,
    /// Grid walls: PART_WALL or PART_LEAF, plus PART_BACK for the face seen from the neighbour (see grid).
    pub part: i32,
    /// World position of the surface point and view depth.
    pub position: Vec3,
    /// View depth of the surface point.
    pub depth: f32,
    /// World normal of the surface.
    pub normal: Vec3,
}
