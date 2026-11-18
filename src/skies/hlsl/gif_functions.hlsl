static const uint4 DATA[@ROWS@] = {
@TABLE@
};

uint Word(uint index)
{
    uint4 row = DATA[index >> 2];
    uint part = index & 3;
    return part == 0 ? row.x : (part == 1 ? row.y : (part == 2 ? row.z : row.w));
}

// One pixel of one frame. x runs round the horizon, so it wraps; y stops at the edges.
float3 Pixel(uint frame, float x, float y)
{
    x = x < 0.0 ? x + WIDE : (x >= WIDE ? x - WIDE : x);
    y = clamp(y, 0.0, HIGH - 1.0);
    uint at = (frame * uint(WIDE * HIGH)) + (uint(y) * uint(WIDE)) + uint(x);
    uint colour = Word((Word(256 + (at >> 2)) >> ((at & 3) * 8)) & 255);
    return float3(colour & 255, (colour >> 8) & 255, (colour >> 16) & 255) / 255.0;
}

// A frame at a point, smoothed between neighbouring pixels along the row only: two reads
// instead of four, since every read into the table is costly. `at` is 0 to 1 across and down.
float3 Frame(uint frame, float2 at)
{
    float spot = (at.x * WIDE) - 0.5;
    float corner = floor(spot);
    float row = floor(at.y * HIGH);
    return lerp(Pixel(frame, corner, row), Pixel(frame, corner + 1.0, row), spot - corner);
}
