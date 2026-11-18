// Lantern Festival: paper sky lanterns rising into a night sky over a still lake, a village's
// lights along the far shore. Everything is fixed in the sky, so it stays put as the camera turns;
// the lanterns only bob gently and their flames flicker, which looks right whichever way the sky
// clock is running.

static const float TAU = 6.2831853;
static const float PI = 3.14159265;

float Hash1(float x) { return frac(sin(x * 127.1) * 43758.5453); }
float Hash2(float2 p) { return frac(sin(dot(p, float2(127.1, 311.7))) * 43758.5453); }
float Hash3(float3 p) { return frac(sin(dot(p, float3(127.1, 311.7, 74.7))) * 43758.5453); }

float Noise2(float2 p)
{
    float2 cell = floor(p);
    float2 f = frac(p);
    f = f * f * (3.0 - (2.0 * f));
    return lerp(lerp(Hash2(cell), Hash2(cell + float2(1.0, 0.0)), f.x),
                lerp(Hash2(cell + float2(0.0, 1.0)), Hash2(cell + float2(1.0, 1.0)), f.x), f.y);
}

// The sky clock eased to a stop at both ends, so nothing driven by it jerks where it turns at zero.
float Ease(float clock)
{
    float c = saturate(clock);
    return c * c * (3.0 - (2.0 * c));
}

// How much of a shape covers a point, from its distance `d` (negative inside), a pixel `px` wide.
float Fill(float d, float px)
{
    return saturate(0.5 - (d / px));
}

// One layer of lanterns. The sky is cut into rings of elevation `band` radians high, each cut
// into as many cells as fit round it, and a share `chance` of the cells hold a lantern `size`
// radians tall, bobbing gently in its cell. Returns the lantern's light in .rgb and how much of
// the point its paper covers in .a.
float4 Lanterns(float phi, float e, float pixel, float clock, float band, float chance, float size, float layer)
{
    float row = floor(e / band);
    float ring = (row + 0.5) * band;
    float slots = max(1.0, floor((TAU * cos(clamp(ring, -1.55, 1.55))) / band));
    float slot_width = TAU / slots;
    float at = (phi + PI) / slot_width;
    float3 id = float3(fmod(floor(at), slots), row, layer * 37.0);
    float2 local = float2(frac(at), frac(e / band));
    // The cell's size in radians, across at this height and up.
    float2 extent = float2(slot_width * max(cos(e), 0.05), band);
    float half_ = 0.5 * size * (0.75 + (0.5 * Hash3(id + 3.3)));
    // The lantern and its glow keep inside the cell as it bobs.
    float2 room = min(float2(half_ * 2.2, half_ * 2.6) / extent, 0.45);
    float2 centre = room + ((1.0 - (2.0 * room)) * float2(Hash3(id + 7.1), Hash3(id + 13.7)));
    float sway = Ease(clock) * TAU;
    centre.y += (0.25 * room.y) * sin((sway * (0.7 + (0.6 * Hash3(id + 5.5)))) + (Hash3(id + 9.9) * TAU));
    centre.x += (0.20 * room.x) * sin((sway * (0.5 + (0.5 * Hash3(id + 6.6)))) + (Hash3(id + 8.8) * TAU));
    float2 d = (local - centre) * extent;
    // A paper lantern: a rounded box a little wider at the top than at its open foot.
    float t = saturate(0.5 + (d.y / (2.0 * half_)));
    float width = half_ * lerp(0.55, 0.78, t);
    float2 q = float2(abs(d.x) - width, abs(d.y) - half_);
    float body = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - (half_ * 0.12);
    float paper = Fill(body, pixel);
    // Its light: brightest low down where the flame is, deep orange toward the top, the flame
    // flickering a little.
    float flicker = 0.85 + (0.15 * Noise2(float2((Ease(clock) * 18.0) + (Hash3(id) * 50.0), layer)));
    float3 colour = lerp(float3(1.00, 0.84, 0.48), float3(0.86, 0.30, 0.06), t) * flicker;
    // Faint ribs of the frame where the lantern is big enough to show them.
    float ribs = 1.0 - (0.18 * saturate((half_ / pixel) - 6.0) * step(0.80, frac((d.x / (2.0 * width)) * 4.0 + 0.5)));
    colour *= ribs;
    // The open foot glows brightest, the flame inside.
    colour += float3(1.0, 0.75, 0.35) * 0.6 * exp(-pow((d.y + (half_ * 0.85)) / (half_ * 0.25), 2.0)) * exp(-pow(d.x / (half_ * 0.35), 2.0));
    // A soft glow round it, fading to nothing before the cell's edge so no seam shows.
    float2 edge = min(local, 1.0 - local);
    float window = smoothstep(0.0, 0.18, edge.x) * smoothstep(0.0, 0.18, edge.y);
    float glow = 0.30 * exp(-max(body, 0.0) / (half_ * 0.5)) * (1.0 - paper) * window;
    float present = step(Hash3(id + 21.3), chance);
    return float4(((colour * paper) + (float3(1.0, 0.55, 0.20) * glow * flicker)) * present, paper * present);
}

// What the scene shows above the lake at direction (phi, e), e at or above zero: the night sky
// warming toward the horizon, faint stars, the far shore with its village lights, and the
// lanterns, three layers of them, near ones big.
float3 Scene(float phi, float e, float pixel, float clock, float3 n)
{
    float up = saturate(sin(e));
    float3 sky = lerp(float3(0.30, 0.14, 0.09), float3(0.07, 0.05, 0.14), smoothstep(0.0, 0.22, up));
    sky = lerp(sky, float3(0.012, 0.014, 0.040), smoothstep(0.18, 0.90, up));
    // A few faint stars, high up.
    float3 grid = n * 140.0;
    float3 box = floor(grid);
    float3 centre = box + 0.5 + ((float3(Hash3(box), Hash3(box + 7.1), Hash3(box + 13.7)) - 0.5) * 0.5);
    float star = step(0.985, Hash3(box + 21.3)) * (1.0 - smoothstep(0.0, 0.15, length(grid - centre)));
    sky += star * 0.30 * smoothstep(0.30, 0.8, up) * float3(0.85, 0.88, 1.0);
    // The far shore: low wooded hills, dark, with the warm lights of a village along the water.
    float ridge = 0.010 + (0.022 * Noise2(float2(phi * 3.0, 1.7))) + (0.008 * Noise2(float2(phi * 40.0, 4.2)));
    float shore = Fill(e - ridge, pixel);
    float2 lamp_at = float2(phi * 220.0, e * 220.0);
    float2 lamp_cell = floor(lamp_at);
    float lamp = step(0.86, Hash2(lamp_cell)) * (1.0 - smoothstep(0.05, 0.3, length(frac(lamp_at) - 0.5))) * step(e, ridge * 0.6);
    float3 land = float3(0.025, 0.018, 0.030) + (float3(1.0, 0.65, 0.30) * lamp * 0.8);
    sky = lerp(sky, land, shore);
    // The lanterns, far to near.
    // Each layer's light comes already weighted by how much its paper covers, so it is laid
    // over what is behind as covered-plus-light.
    float fade = 1.0 - (0.6 * smoothstep(0.7, 1.4, e));
    float4 far_ = Lanterns(phi, e, pixel, clock, 0.030, 0.22 * LANTERNS * fade, 0.0085, 1.0);
    sky = (sky * (1.0 - far_.a)) + far_.rgb;
    float4 mid = Lanterns(phi + 0.37, e, pixel, clock, 0.070, 0.20 * LANTERNS * fade, 0.020, 2.0);
    sky = (sky * (1.0 - mid.a)) + mid.rgb;
    float4 close = Lanterns(phi + 0.91, e, pixel, clock, 0.160, 0.15 * LANTERNS * fade, 0.046, 3.0);
    sky = (sky * (1.0 - close.a)) + close.rgb;
    // A warm haze low over the water from all the lanterns.
    sky += float3(0.30, 0.13, 0.05) * 0.35 * exp(-e / 0.06);
    return sky;
}
