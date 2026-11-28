/*
 * Pixelraum - pixel-art 3D software renderer. C interface.
 *
 * The library draws into its own memory buffers; it opens no window and needs no graphics API.
 * The game shows the pixels itself (e.g. by uploading pr_pixels_rgba() into an SDL texture).
 *
 * One frame:
 *     pr_begin(r, &camera);
 *     pr_draw_grid(r, grid);          any number of draw calls, any order
 *     pr_draw_sprite(r, &sprite);
 *     pr_end(r);                      light, palette, dithering, outlines
 *     pixels = pr_pixels_rgba(r);     width * height * 4 bytes, valid until the next pr_end/resize
 *
 * World axes: x east, y up, z south. Camera yaw 0 looks north (-z), positive yaw turns right,
 * positive pitch looks up. Angles in radians, field of view in degrees.
 * Colours as 0xRRGGBB. Texture ids are ints >= 0; -1 means "none".
 * All effects in PR_Options are off by default: without them, texture pixels come out unchanged.
 *
 * Keep in sync with src/lib.rs (knorke-pixel-c; origin: pixelraum @ 81927d8, include/pixelraum.h, MIT). Works with C89 compilers that have <stdint.h> (gcc -std=gnu89).
 */
#ifndef PIXELRAUM_H
#define PIXELRAUM_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Define PR_USE_DLL when linking against pixelraum.dll on Windows. */
#if defined(PR_USE_DLL) && defined(_WIN32)
#define PR_API __declspec(dllimport)
#else
#define PR_API
#endif

#define PR_VERSION 0x000900u /* 0xMMmmpp; pass it to pr_renderer_create */

typedef struct PR_Renderer PR_Renderer;
typedef struct PR_Grid PR_Grid;

/* Look switches. Fill with pr_default_options(), then change what you need. */
typedef struct PR_Options {
    uint32_t struct_size;    /* sizeof(PR_Options), set by pr_default_options; pr_set_options rejects other sizes */
    uint32_t background;     /* colour where nothing is drawn and no sky is set */
    /* Lighting (see pr_set_sun, pr_set_ambient, pr_light_create) */
    int32_t lighting;        /* 1: lighting pass; 0 = unchanged images.
                                Legacy sun_dir/sun_strength are ignored while lighting is 1.
                                Solid grid cells have no faces of their own: they cast sun shadows
                                through the walls of their open neighbours, so give those walls a
                                texture and a ceiling_height as tall as the building. */
    float shadow_size;       /* half-size of the sun shadow box in world units; 0 = draw_distance, else 32 */
    int32_t shadow_res;      /* sun shadow map size in texels, 1..4096, default 512 */
    int32_t point_shadow_res;/* point light shadow cube face size in texels, 1..4096, default 128 */
    float shadow_bias;       /* shadow bias in world units, default 0.02 */
    float shadow_soft;       /* soft shadow edges: penumbra width in world units, 0 = hard (default).
                                Nine taps in a fixed light-space pattern, stepped to 0, 1/3, 2/3, 1 with
                                an ordered 4x4 dither between steps; sun only. Clamped to 0..64, NaN = 0. */
    float point_shadow_soft; /* the same for shadowed point lights, default 0 (costs about 4x per light) */
    int32_t light_levels;    /* light steps (toon look); 0 = smooth */
    float sun_dir[3];        /* legacy shading by surface normal (without lighting): direction the light travels */
    float sun_strength;      /* 0..1, 0 = off */
    /* Distance fog */
    float fog_start;         /* distance darkening: full light up to fog_start ... */
    float fog_end;           /* ... fog_min from fog_end on; off if fog_end <= fog_start */
    float fog_min;
    uint32_t fog_color;      /* colour the distance fades to; 0 = black like Doom */
    /* Palette and dithering */
    int32_t quantize;        /* 1: reduce the RGBA output to the palette (pr_set_palette) */
    int32_t dither;          /* 1: 4x4 Bayer dithering before the palette reduction */
    float dither_strength;   /* in colour steps 0..255, default 24 */
    float snap;              /* snap the camera to this world step (no shimmer while moving); 0 = off */
    /* Outlines */
    int32_t outlines;        /* 1: outlines from depth jumps and normal creases */
    float outline_depth;     /* relative depth jump that counts as an edge, default 0.15 */
    float outline_normal;    /* crease if the dot product of neighbour normals is below this, default 0.5 */
    uint32_t outline_color;
    /* Picking */
    int32_t pick;            /* 1: record per pixel what was drawn, for pr_pick(); 0 = no cost. Never changes the image. */
    /* Distance and texture levels */
    float draw_distance;     /* grid cells farther than this are skipped; 0 = no limit */
    int32_t texture_lod;     /* 1: far surfaces use smaller texture levels that keep dark lines (joints) */
    float lod_bias;          /* > 0 picks smaller levels earlier, < 0 later; default 0 */
    int32_t lod_dither;      /* 1 (default): ordered dither between two levels (soft band); 0: nearest level, hard boundary */
    int32_t face_lod;        /* 1: one level per grid face (no change of picture within a wall); 0: per pixel */
    float contour_falloff;   /* textures with a contour mask: 0 (default) = a masked texel always wins its block, so
                                far joints cover every texel they touch; 1 = a joint keeps about its true area share
                                on far levels (lines thinned, not erased); between blends. Changing it rebuilds the
                                levels of masked textures on the next frame. */
    /* Speed */
    int32_t threads;         /* worker threads for shadow maps and shading; 0 = all cores, at most 8 (default), 1 = none.
                                The image is the same for any count. */
    /* Shadows of see-through things */
    int32_t shadow_stipple;  /* 1 (default): PR_BLEND_ALPHA sprites, triangle lists and grid walls with alpha < 1 throw
                                a stippled shadow (an ordered pattern of shadow map texels fixed in light space covers
                                the share given by alpha in eighths; 1/2 = checkered). PR_BLEND_ADD throws none.
                                0: see-through things throw no shadow (as before). */
} PR_Options;

typedef struct PR_Camera {
    float x, y, z;           /* eye position */
    float yaw, pitch;        /* radians */
    float fov_y;             /* vertical field of view in degrees */
    float near_clip;         /* near clip distance, e.g. 0.05 */
    /* Lens shift: moves the projection centre (the point straight ahead) without turning the camera.
     * shift_x in fractions of the image width, + = to the right; shift_y in fractions of the image height,
     * + = up. 0 = centre; shift_y = 0.25 puts the horizon of a level camera a quarter of the image above
     * the centre. The image is the matching crop of a larger one (same field of view per pixel). */
    float shift_x, shift_y;
} PR_Camera;

/* Sides of a grid cell */
#define PR_NORTH 0 /* -z */
#define PR_EAST  1 /* +x */
#define PR_SOUTH 2 /* +z */
#define PR_WEST  3 /* -x */
/* Further faces, reported by pr_pick() and pr_grid_raycast() */
#define PR_SIDE_FLOOR   4
#define PR_SIDE_CEILING 5
#define PR_SIDE_STEP    6 /* step face, or the side of a solid neighbour */

/* Door modes (PR_Wall.door_mode) */
#define PR_DOOR_UP          0 /* the leaf slides up; its top part hides in the frame (without a leaf: in the ceiling) */
#define PR_DOOR_SLIDE_LEFT  1 /* slides to the left within the wall plane, hidden beyond the leaf rectangle */
#define PR_DOOR_SLIDE_RIGHT 2
#define PR_DOOR_SWING_LEFT  3 /* turns about a vertical hinge at the leaf's left edge */
#define PR_DOOR_SWING_RIGHT 4 /* hinge at the right edge */

/* Wall coordinates: u runs 0..1 along the wall, left to right as seen from inside the wall's own cell;
 * heights are fractions 0 (floor) .. 1 (ceiling) of the wall at that u (sloped cells: a trapezoid).
 * Start from pr_default_wall() (or pr_default_cell()): a zeroed PR_Wall is texture 0, back texture 0 and alpha 0
 * (invisible). A zeroed leaf rectangle (leaf_u0 = leaf_u1 = 0, leaf_top = 0) means the whole wall. */
typedef struct PR_Wall {
    int32_t texture;         /* -1 = no wall. With a separate leaf: the frame, drawn only around the leaf rectangle */
    float open;              /* door: 0 closed .. 1 open, see door_mode */
    float alpha;             /* opacity of frame and leaf, default 1; < 1 = see-through (PR_BLEND_ALPHA), no shadow,
                              * not pickable; 0 = invisible */
    int32_t back_texture;    /* -1 (default) = one-sided. Else the same wall is also drawn facing the neighbour cell,
                              * with this texture, not mirrored (u flipped, the picture reads left to right from there
                              * too), lit with the neighbour's light. Frame and planar leaf get back faces as well.
                              * Skipped where the neighbour has its own wall on this edge (the neighbour's wall wins). */
    int32_t leaf_texture;    /* -1 (default) = no separate leaf: the whole wall is the door leaf (texture), the leaf
                              * rectangle is ignored, picks and raycasts report PR_PART_WALL. Else the leaf picture,
                              * stretched over the leaf rectangle (u 0..1 left to right, v 0 at the top). */
    int32_t door_mode;       /* PR_DOOR_*, default PR_DOOR_UP */
    float leaf_u0, leaf_u1;  /* leaf rectangle across the wall, default 0..1; leaf_u1 <= leaf_u0 = 0..1 */
    float leaf_top;          /* height of the leaf's top edge as a fraction of the wall height (bottom = floor),
                              * default 1; <= 0 means 1 */
    float swing_angle;       /* swing modes: angle in degrees at open = 1, default 90, clamped to -180..180.
                              * Positive swings into this cell, negative into the neighbour. A swung leaf is a
                              * rectangle with the floor and leaf-top heights at its hinge (also in sloped cells)
                              * and is drawn from both sides; its back shows the same picture mirrored (like a
                              * painted board), except a whole-wall leaf with a back_texture shows that one.
                              * Closed (open 0) it is an ordinary wall face. */
} PR_Wall;

/* Cell (x, z) covers [x, x+1] * cell_size by [z, z+1] * cell_size. Heights are in cell sizes.
 * A wall faces into its own cell; give it a back_texture (or set a wall on both cells) to see it from both sides.
 * Door leaves, frames and their back faces cast shadows with lighting (the back face shares its front's shadow).
 * ceiling_height <= floor_height makes the cell solid.
 *
 * Storeys: a grid has `levels` cells per column (pr_grid_create). The cell at level k is one open span from
 * its floor to its ceiling; a solid cell is no span. Everything outside the spans of a column is solid.
 * Spans are used bottom to top: where a span's floor is below the ceiling of the open span under it, it is
 * raised to that ceiling (per corner); a span that lies wholly inside or under a lower one is ignored.
 * Neighbours go by height, not by level: an edge shows step faces where this span's height is not open in
 * the neighbour column. A step piece below a neighbour span (its floor slab) uses that span's side_texture
 * (else its floor_texture); the piece above the neighbour's topmost span uses the topmost span's texture.
 * An untextured floor or ceiling is a hole into the span above/below. Walls belong to their span and cover
 * its height on the edge; a back face looks into the neighbour span with the most height in common.
 * A house with a gable roof: level 0 the room (textured ceiling), level 1 a span on top whose sloped floor
 * (floor_corners) is the roof; the street cells around show the gable triangles as step faces. */
typedef struct PR_Cell {
    float floor_height;      /* default 0 */
    float ceiling_height;    /* default 1 */
    int32_t floor_texture;
    int32_t ceiling_texture; /* -1 = open sky */
    int32_t side_texture;    /* step faces towards lower/higher neighbours; -1 = floor texture */
    float light;             /* 0..1, default 1 */
    PR_Wall walls[4];        /* indexed by PR_NORTH .. PR_WEST */
    /* Slopes: offsets in cell sizes added to floor_height / ceiling_height at the corners NW, NE, SE, SW
     * (NW = smallest x and z; default 0). Sloped surfaces are two triangles split along NW-SE; walls and step
     * faces follow the corner heights. Where a corner's floor >= its ceiling, the ceiling is clamped to the
     * floor there; all four corners like that make the cell solid. Raycasts and rain culling use the slopes. */
    float floor_corners[4];
    float ceiling_corners[4];
} PR_Cell;

/* Blend modes for sprites and triangle lists. Transparent surfaces are blended after the opaque scene is lit,
 * far to near (equal depth: later draw on top); alpha (times texel alpha) is stepped to 1/8. They write no depth,
 * cast no shadow and are not pickable. ALPHA is lit like opaque surfaces; ADD adds its colour (ignores light). */
#define PR_BLEND_NONE  0 /* opaque, alpha ignored; texel alpha < 128 is a hole */
#define PR_BLEND_ALPHA 1
#define PR_BLEND_ADD   2

#define PR_SPRITE_BILLBOARD 1u /* always faces the camera (turns around the vertical axis) */
#define PR_SPRITE_FLIP_X    2u /* mirror the image */
#define PR_SPRITE_NO_SHADOW 4u /* casts no shadow (only matters with PR_Options.lighting) */

typedef struct PR_Sprite {
    float x, y, z;           /* bottom centre in the world */
    float width, height;     /* world units */
    int32_t texture;         /* the current animation frame; see pr_anim_frame() */
    uint32_t flags;
    float yaw;               /* facing without PR_SPRITE_BILLBOARD; 0 faces south (+z) */
    float light;             /* 0..1 */
    uint32_t tag;            /* reported by pr_pick() */
    float alpha;             /* 0..1, used with blend != PR_BLEND_NONE */
    int32_t blend;           /* PR_BLEND_*; set 0 for opaque */
} PR_Sprite;

/* Sun: direction the light travels, colour 0xRRGGBB, strength 0..1. */
typedef struct PR_Sun {
    float dir[3];
    uint32_t color;
    float strength;
    int32_t shadows;         /* 1 = casts shadows */
} PR_Sun;

/* Point light (torch), see pr_light_create. radius in world units; flicker 0..1 varies the strength over time (pr_set_time),
 * seed makes each torch flicker differently. */
typedef struct PR_Light {
    float x, y, z;
    uint32_t color;
    float strength, radius, flicker;
    uint32_t seed;
    int32_t shadows;         /* 1 = casts shadows (cube shadow map, costs time) */
} PR_Light;

typedef struct PR_Vertex {
    float x, y, z;
    float u, v;              /* texture coordinates, 0..1 per repeat; v = 0 is the top row */
} PR_Vertex;

PR_API uint32_t pr_version(void);

/* Renderer: internal resolution, e.g. 320 x 200. Pass PR_VERSION as header_version: NULL unless its major
 * and minor match the library (patch may differ), so a stale header or library fails loudly. NULL on bad sizes. */
PR_API PR_Renderer *pr_renderer_create(uint32_t header_version, int32_t width, int32_t height);
PR_API void pr_renderer_destroy(PR_Renderer *r);
PR_API int32_t pr_renderer_resize(PR_Renderer *r, int32_t width, int32_t height); /* 1 = ok */

PR_API void pr_default_options(PR_Options *out);
/* 1 = ok; 0 = rejected (NULL, or options->struct_size != the library's sizeof(PR_Options)); options unchanged. */
PR_API int32_t pr_set_options(PR_Renderer *r, const PR_Options *options);
PR_API void pr_get_options(PR_Renderer *r, PR_Options *out);

/* Output palette (RGB triples, 1..256 colours). Enables pr_pixels_indexed(). count 0 removes it.
 * Colours that are exactly in the palette map to their first index. Returns 1 on success. */
PR_API int32_t pr_set_palette(PR_Renderer *r, const uint8_t *rgb, int32_t count);

/* Textures. RGBA: alpha < 128 is transparent. Indexed: palette_rgb has palette_count RGB triples,
 * transparent_index -1 for none. Return the texture id, or -1 on error. Textures are copied. */
PR_API int32_t pr_texture_rgba(PR_Renderer *r, int32_t width, int32_t height, const uint8_t *rgba);
PR_API int32_t pr_texture_indexed(PR_Renderer *r, int32_t width, int32_t height, const uint8_t *pixels,
                           const uint8_t *palette_rgb, int32_t palette_count, int32_t transparent_index);
/* Replace an existing texture (same id). Return 1 on success. */
PR_API int32_t pr_texture_update_rgba(PR_Renderer *r, int32_t id, int32_t width, int32_t height, const uint8_t *rgba);
PR_API int32_t pr_texture_update_indexed(PR_Renderer *r, int32_t id, int32_t width, int32_t height,
                                  const uint8_t *pixels, const uint8_t *palette_rgb, int32_t palette_count,
                                  int32_t transparent_index);

/* Sky shown where nothing is drawn: the texture wraps `repeats` times around the horizon,
 * its top row straight up, its bottom row at the horizon. texture -1 removes it. */
/* Far level of its own for texture id (any size, e.g. a hand-drawn smaller version of the picture),
 * used with PR_Options.texture_lod. The first call drops the levels built at load time; replacing the
 * texture builds them again. 1 = ok. */
PR_API int32_t pr_texture_add_level_indexed(PR_Renderer *r, int32_t id, int32_t width, int32_t height,
                                     const uint8_t *pixels, const uint8_t *palette_rgb, int32_t palette_count,
                                     int32_t transparent_index);
/* Contour mask for texture id: one byte per texel (width x height = the texture size), 0 = normal,
 * 1..255 = importance. When the far levels are built, the texel with the highest value in each block
 * wins with its real colour, so masked lines (joints, seams, rims) survive at a distance. NULL removes
 * the mask. Rebuilds the levels; replacing the texture drops the mask. 1 = ok. */
PR_API int32_t pr_texture_set_contour(PR_Renderer *r, int32_t id, const uint8_t *mask, int32_t width, int32_t height);
/* Mask derived from the picture: texels clearly darker than their 3x3 neighbourhood get contrast x
 * strength (0 removes the mask). 1 = ok. */
PR_API int32_t pr_texture_contour_auto(PR_Renderer *r, int32_t id, float strength);
/* Mask for a caller-made far level (pr_texture_add_level_indexed; level 1 = widest, 0 = the texture
 * itself). Without one, such a level gets the base mask reduced to its size; NULL goes back to that. 1 = ok. */
PR_API int32_t pr_texture_set_level_contour(PR_Renderer *r, int32_t id, int32_t level, const uint8_t *mask,
                                            int32_t width, int32_t height);
PR_API void pr_set_sky(PR_Renderer *r, int32_t texture, float repeats);
/* Animated texture: a new texture id that draws frames[pr_anim_frame(time, fps, count)] at the time of
 * pr_set_time() (quantise the time yourself for a choppy look). Usable wherever a texture id is (walls,
 * sprites, meshes, sky); shadows follow the shown frame. Frames must be existing textures that are not
 * animations themselves. Returns the id, or -1 on error (count <= 0, bad frame, fps <= 0).
 * pr_texture_update_* on the animation id turns it into a plain texture again. */
PR_API int32_t pr_texture_anim(PR_Renderer *r, const int32_t *frames, int32_t count, float fps);
/* Scroll texture id by (du, dv) texture coordinates per second of pr_set_time() time (offset = fraction of
 * du * time). Faces drawn with it repeat the texture, also faces that normally clamp (sprites).
 * On the sky, u runs along the horizon (clouds). Billboard sprite shadows scroll with the sprite.
 * Scrolled faces that cast shadows rebuild the shadow maps they reach every frame. 0, 0 stops it. 1 = ok, 0 = unknown id. */
PR_API int32_t pr_texture_scroll(PR_Renderer *r, int32_t id, float du, float dv);

/* All pr_draw_* calls are ignored outside pr_begin .. pr_end (they change neither the results of the
 * last frame nor the next one). */
PR_API void pr_begin(PR_Renderer *r, const PR_Camera *camera);
PR_API void pr_draw_grid(PR_Renderer *r, const PR_Grid *grid);
PR_API void pr_draw_sprite(PR_Renderer *r, const PR_Sprite *sprite);
/* Triangle list, count = number of vertices (3 per triangle). Front faces are clockwise as seen
 * on screen. Flat-shaded; texture coordinates repeat. cull = 1 skips back faces. */
PR_API void pr_draw_triangles(PR_Renderer *r, const PR_Vertex *vertices, int32_t count, int32_t texture,
                       float light, int32_t cull);
/* Like pr_draw_triangles; cast_shadow = 0 keeps the mesh out of the shadow maps, tag is reported by pr_pick().
 * blend PR_BLEND_ALPHA / PR_BLEND_ADD with alpha 0..1 draws it see-through (PR_BLEND_NONE: alpha ignored). */
PR_API void pr_draw_triangles_ex(PR_Renderer *r, const PR_Vertex *vertices, int32_t count, int32_t texture,
                          float light, int32_t cull, int32_t cast_shadow, uint32_t tag, float alpha, int32_t blend);
PR_API void pr_end(PR_Renderer *r);

/* Lighting, used with PR_Options.lighting = 1. Everything stays until changed (also across frames)
 * and may be called at any time outside pr_begin .. pr_end as well. */
PR_API void pr_set_sun(PR_Renderer *r, const PR_Sun *sun);              /* NULL = no sun */
PR_API void pr_set_ambient(PR_Renderer *r, uint32_t color, float strength);
PR_API void pr_set_sky_tint(PR_Renderer *r, uint32_t color, float strength); /* tints the sky texture */
PR_API void pr_set_time(PR_Renderer *r, double time);                   /* seconds, drives torch flicker */
/* Persistent point lights. Ids are >= 0 and never reused. Any number may exist; each frame uses at most 16:
 * those whose sphere reaches the visible area, nearest to the camera first (ties by lower id). */
PR_API int32_t pr_light_create(PR_Renderer *r, const PR_Light *light);  /* id, or -1 on error */
PR_API int32_t pr_light_set(PR_Renderer *r, int32_t id, const PR_Light *light); /* 1 = ok, 0 = unknown id */
PR_API int32_t pr_light_destroy(PR_Renderer *r, int32_t id);            /* 1 = ok, 0 = unknown id */

/* Particles: camera-facing squares, size in world units, depth-tested, no shadows. Not pickable: pr_pick
 * sees through them and reports the surface behind (or nothing).
 * Blended when alpha < 1 (like PR_BLEND_ALPHA) or with PR_PARTICLE_ADDITIVE, else opaque. Lit by ambient + sun
 * without shadows unless PR_PARTICLE_EMISSIVE. At most 4096 particles per frame (pr_draw_particles and emitters
 * together); later ones are dropped in drawing order. */
#define PR_PARTICLE_EMISSIVE 1u /* ignores light */
#define PR_PARTICLE_ADDITIVE 2u /* adds colour * alpha to what is behind */
typedef struct PR_Particle {
    float x, y, z, size;
    uint32_t color;          /* 0xRRGGBB, tints the texture */
    int32_t texture;         /* -1 = solid square */
    uint32_t flags;          /* PR_PARTICLE_* */
    float alpha;             /* 0..1 */
} PR_Particle;
PR_API void pr_draw_particles(PR_Renderer *r, const PR_Particle *p, int32_t count); /* between begin/end; ignored otherwise */

/* Emitters: persistent, simulated from pr_set_time alone (no per-frame state): particle i is born at i / rate,
 * so the same time gives the same picture. SPARKS (emissive, additive) rise and fade, SMOKE rises, grows and fades;
 * both start inside the box around x, y, z. RAIN and SNOW fill the box (pass the camera position), fall with the
 * wind and are left out under textured ceilings, below floors and in solid cells of the grid given to
 * pr_draw_emitters. speed <= 0 and size <= 0 pick defaults of the kind. */
#define PR_EMIT_SPARKS 0
#define PR_EMIT_SMOKE 1
#define PR_EMIT_RAIN 2
#define PR_EMIT_SNOW 3
typedef struct PR_Emitter {
    int32_t kind;            /* PR_EMIT_* */
    float x, y, z;           /* source / box centre */
    float size_x, size_y, size_z; /* box size */
    float rate;              /* particles per second */
    float speed;             /* rise / fall speed, world units per second */
    float size;              /* particle size, world units */
    uint32_t color;
    int32_t texture;         /* -1 = solid squares */
    float wind_x, wind_z;    /* drift, world units per second */
    uint32_t seed;
} PR_Emitter;
/* Ids are >= 0 and never reused. */
PR_API int32_t pr_emitter_create(PR_Renderer *r, const PR_Emitter *e);    /* id, or -1 on error */
PR_API int32_t pr_emitter_set(PR_Renderer *r, int32_t id, const PR_Emitter *e); /* 1 = ok, 0 = unknown id */
PR_API int32_t pr_emitter_destroy(PR_Renderer *r, int32_t id);            /* 1 = ok, 0 = unknown id */
PR_API void pr_draw_emitters(PR_Renderer *r, const PR_Grid *grid);         /* between begin/end (ignored otherwise); grid may be NULL */

/* Results of the last pr_end(). Row-major, width * height entries, valid until next pr_end/resize. */
PR_API const uint8_t *pr_pixels_rgba(PR_Renderer *r);    /* 4 bytes per pixel, alpha 255 */
PR_API const uint8_t *pr_pixels_indexed(PR_Renderer *r); /* palette index per pixel; NULL without palette */
PR_API const float *pr_depth(PR_Renderer *r);            /* view depth per pixel; +infinity where empty */
PR_API const float *pr_normals(PR_Renderer *r);          /* world normal x, y, z per pixel; 0 where empty */

/* Picking: what the last frame drew under pixel (x, y). Needs PR_Options.pick = 1 for that frame. */
#define PR_PICK_NONE   0
#define PR_PICK_GRID   1 /* cell_x, cell_z, side (PR_NORTH .. PR_WEST, PR_SIDE_*) are set, else -1 */
#define PR_PICK_SPRITE 2
#define PR_PICK_MESH   3
/* Part of a wall (PR_Pick.part, PR_GridHit.part), bit flags */
#define PR_PART_WALL 0 /* the wall, the frame around a separate leaf, or a whole-wall door; 0 for non-wall faces */
#define PR_PART_LEAF 1 /* the separate leaf (leaf_texture >= 0) */
#define PR_PART_BACK 2 /* pr_pick only: the face seen from the neighbour (back texture, back of a leaf) */
typedef struct PR_Pick {
    int32_t kind;            /* PR_PICK_* */
    uint32_t tag;            /* sprite or mesh tag; 0 for grid faces */
    int32_t cell_x, cell_z, side;
    float x, y, z;           /* world position of the surface point */
    float depth;             /* view depth */
    float nx, ny, nz;        /* world normal */
    int32_t part;            /* PR_PART_* for walls (cell and side are always the wall's own) */
    int32_t level;           /* storey of the grid cell, -1 for sprites and meshes */
} PR_Pick;
/* 1 = something there; else 0 and *out is zeroed (also off screen and without PR_Options.pick). */
PR_API int32_t pr_pick(PR_Renderer *r, int32_t x, int32_t y, PR_Pick *out);

/* Counters of the last pr_end(). */
typedef struct PR_Stats {
    uint32_t dropped_fragments;   /* see-through fragments over the cap of 2^20 per frame, not drawn */
    uint32_t dropped_particles;   /* particles over the budget of 4096 per frame, not drawn */
    uint32_t lights_used;         /* point lights used (at most 16) */
    uint32_t shadow_maps_rebuilt; /* sun map and point light cube maps rebuilt (a cube counts one) */
} PR_Stats;
/* 1 = ok; 0 on a NULL pointer (*out zeroed if out is not NULL). */
PR_API int32_t pr_stats(PR_Renderer *r, PR_Stats *out);

/* Grid maps. cell_size in world units. levels = storeys per column, 1..64 (width * depth * levels at most
 * 2^24). Level 0 starts as empty rooms without textures, the levels above start solid (no span).
 * NULL on bad arguments. */
PR_API PR_Grid *pr_grid_create(int32_t width, int32_t depth, int32_t levels, float cell_size);
PR_API int32_t pr_grid_levels(const PR_Grid *grid); /* 0 for NULL */
PR_API void pr_grid_destroy(PR_Grid *grid);
PR_API void pr_default_cell(PR_Cell *out);  /* also fills all four walls like pr_default_wall */
PR_API void pr_default_wall(PR_Wall *out);  /* no wall: texture -1, alpha 1, no back, no leaf, PR_DOOR_UP, 90 degrees */
/* 1 = ok, 0 for x, z or level out of range */
PR_API int32_t pr_grid_set_cell(PR_Grid *grid, int32_t x, int32_t z, int32_t level, const PR_Cell *cell);
PR_API int32_t pr_grid_get_cell(const PR_Grid *grid, int32_t x, int32_t z, int32_t level, PR_Cell *out);
/* Sets texture and open of one wall, keeps its other fields (alpha, back, leaf, door mode). 1 = ok. */
PR_API int32_t pr_grid_set_wall(PR_Grid *grid, int32_t x, int32_t z, int32_t level, int32_t side, int32_t texture,
                                float open);

/* First grid face along a ray, sprites and meshes ignored. Walls block from both sides (cell and side
 * of the wall's own cell are reported). Doors: the frame (wall outside the leaf rectangle) always blocks; the
 * leaf rectangle blocks where the leaf still is (UP / SLIDE: the still covered part), a swung leaf blocks where
 * the ray meets its rectangle, the opening it leaves lets the ray through. Texture holes are ignored. Textured floors and ceilings count;
 * an untextured one lets the ray on into the span of the same column it meets there (storeys). The ray starts in
 * the span containing the origin (else the nearest). Crossing an edge it enters the neighbour span that contains
 * its height; with none (below a floor, above a ceiling, a slab between spans, a solid column) it is PR_SIDE_STEP.
 * Leaving the map, or nothing within max_dist (world units), is a miss. */
typedef struct PR_GridHit {
    int32_t hit;             /* 1 = hit */
    int32_t cell_x, cell_z, side; /* PR_NORTH .. PR_WEST or PR_SIDE_* */
    float distance;          /* world units along the ray */
    float x, y, z;           /* hit point */
    int32_t part;            /* PR_PART_WALL or PR_PART_LEAF */
    int32_t level;           /* storey of the cell (PR_SIDE_STEP: the span the ray was in) */
} PR_GridHit;
/* 1 = hit; else 0 and *out is zeroed. dir need not be normalised. See-through walls (alpha < 1) block
 * the ray like opaque ones. */
PR_API int32_t pr_grid_raycast(const PR_Grid *g, const float origin[3], const float dir[3], float max_dist,
                               PR_GridHit *out);

/* Fixed frame rate animation ("choppy" on purpose): frame of `count` shown at `time` seconds. */
PR_API int32_t pr_anim_frame(double time, float fps, int32_t count);
PR_API double pr_quantize_time(double time, float fps);

/* Backdrops: background layers behind all geometry (sky, mountains, forest, sun, moon). Drawn only where no
 * opaque geometry was drawn, back to front by order (lower = further back, ties: lower id further back),
 * all behind the pr_set_sky picture's layer, which stays the backmost. No light, no shadows, no depth, not
 * pickable, no outlines. Angles in degrees; azimuth 0 = north (-z), 90 = east (+x); elevation 0 = horizon.
 * Band: for the pixel's ray with azimuth a and elevation e,
 *   u = (a - azimuth) / 360 * repeats (+ scroll), v = 1 - (e - elevation) / height (+ scroll);
 *   v = 0 is the top texel row at elevation + height, the bottom edge sits at elevation. Outside 0 <= v < 1 the
 *   band is a hole unless PR_BACKDROP_CLAMP_TOP / _BOTTOM repeat the top / bottom row. Glued to the horizon
 *   for any pitch and lens shift. pr_set_sky(t, rep) equals a band with elevation 0, height 90, both clamps,
 *   PR_BACKDROP_TINT (and texel alpha ignored).
 * Spot: upright image centred on the direction (azimuth, elevation), drawn at a whole-number scale s >= 1
 *   (each texel s x s pixels): s = round(size * focal / texture height), size in degrees; size 0 = s 1.
 * blend PR_BLEND_NONE: texel alpha < 128 is a hole; PR_BLEND_ALPHA / PR_BLEND_ADD: alpha * texel alpha in 1/8
 * steps. Animated textures and scrolling (pr_texture_scroll, bands only) apply. */
#define PR_BACKDROP_BAND 0
#define PR_BACKDROP_SPOT 1
#define PR_BACKDROP_CLAMP_TOP    1u /* band: top row extends upward */
#define PR_BACKDROP_CLAMP_BOTTOM 2u /* band: bottom row extends downward */
#define PR_BACKDROP_TINT         4u /* pr_set_sky_tint applies (with lighting on) */
#define PR_BACKDROP_FOG          8u /* mixed towards fog_color like an infinitely far surface (fog_min) */
typedef struct PR_Backdrop {
    int32_t kind;            /* PR_BACKDROP_BAND / PR_BACKDROP_SPOT */
    int32_t texture;
    int32_t order;           /* lower = further back */
    uint32_t flags;          /* PR_BACKDROP_* */
    float azimuth;           /* band: azimuth of u = 0; spot: direction */
    float elevation;         /* band: bottom edge; spot: centre */
    float height;            /* band: angular height, > 0 */
    float repeats;           /* band: texture repeats per 360 degrees (<= 0 = 1) */
    float size;              /* spot: wanted angular height (whole-number texel scale) */
    int32_t blend;           /* PR_BLEND_* */
    float alpha;             /* with PR_BLEND_ALPHA / PR_BLEND_ADD */
} PR_Backdrop;
/* Ids are >= 0 and never reused. Invalid: unknown kind or texture, band height <= 0, numbers not finite. */
PR_API int32_t pr_backdrop_create(PR_Renderer *r, const PR_Backdrop *b);    /* id, or -1 on error */
PR_API int32_t pr_backdrop_set(PR_Renderer *r, int32_t id, const PR_Backdrop *b); /* 1 = ok, 0 = unknown id / invalid */
PR_API int32_t pr_backdrop_destroy(PR_Renderer *r, int32_t id);            /* 1 = ok, 0 = unknown id */

#ifdef __cplusplus
}
#endif

#endif /* PIXELRAUM_H */
