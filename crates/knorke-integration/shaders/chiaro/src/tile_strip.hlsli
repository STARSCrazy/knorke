// Origin: chiaro @ 9bc5b9b, shaders/src/tile_strip.hlsli, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Edge strips of tile cells (E29): with cells_edges a solid cell is an obstacle only in strips along the edges named
// by the alpha bits of its texel (1 local x low, 2 x high, 4 y low, 8 y high); alpha 255 is the whole cell.
// Strip: xy strip width as a share of the cell per axis, zw columns and rows; all 0 without cells_edges, then
// every cell is whole and nothing here changes the result.
#ifndef CHI_TILE_STRIP_HLSLI
#define CHI_TILE_STRIP_HLSLI

// True if the mask texel at uv lies outside every strip of its cell. Call before any discard: the strip is at least
// one mask texel thick (fwidth of the cell coordinates), so a ray cannot step over it.
bool OutsideStrips(float2 uv, float alpha, float4 strip)
{
    float2 f = uv * strip.zw;
    float2 width = max(strip.xy, fwidth(f));
    uint edges = uint(round(alpha * 255.0));
    if (strip.z <= 0.0 || edges > 15u) {
        return false; // whole cell
    }
    float2 inCell = frac(f);
    bool strips = ((edges & 1u) != 0u && inCell.x < width.x) || ((edges & 2u) != 0u && inCell.x > 1.0 - width.x) ||
                  ((edges & 4u) != 0u && inCell.y < width.y) || ((edges & 8u) != 0u && inCell.y > 1.0 - width.y);
    return !strips;
}

#endif
