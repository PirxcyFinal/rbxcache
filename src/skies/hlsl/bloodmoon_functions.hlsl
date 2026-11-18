// Blood Moon: a huge red moon low in a dark sky, thin clouds drifting across it, bare trees
// along the horizon, fog low on the ground and bats flapping. Everything is fixed in the sky;
// the bats beat their wings in place and the clouds sway, which looks right whichever way the
// sky clock runs.

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

// Layers of noise, 0 to 1.
float Cloud(float2 p)
{
    float sum = 0.0;
    float weight = 0.5;
    for (int octave = 0; octave < 4; octave++)
    {
        sum += weight * Noise2(p);
        p = (p * 2.03) + 19.19;
        weight *= 0.5;
    }
    return sum;
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

// The distance to the line from `a` to `b`.
float Segment(float2 p, float2 a, float2 b)
{
    float2 pa = p - a;
    float2 ba = b - a;
    return length(pa - (ba * saturate(dot(pa, ba) / dot(ba, ba))));
}

// Where the moon hangs: low, opposite the game's own sun, or to the north-east if there is none.
float3 MoonDirection()
{
    float3 lamp = -G[16].xyz;
    float bearing = (dot(lamp.xz, lamp.xz) > 1e-6) ? (atan2(lamp.z, lamp.x) + PI) : 2.3;
    float rise = 0.20;
    return float3(cos(bearing) * cos(rise), sin(rise), sin(bearing) * cos(rise));
}

// A bare tree at the place `x` (0 to 1 across its share of the horizon, base at the bottom),
// `h` tall: a trunk forking into crooked branches and twigs. Distance in the same units as `p`.
float Tree(float2 p, float h, float seed)
{
    float d = Segment(p, float2(0.0, 0.0), float2(0.0, h * 0.45)) - (h * 0.035);
    // Branches fork off the trunk at a few heights, then fork again.
    [unroll]
    for (int k = 0; k < 4; k++)
    {
        float side = ((k & 1) == 0) ? 1.0 : -1.0;
        float at = h * (0.30 + (0.13 * k));
        float lean = side * (0.35 + (0.35 * Hash1(seed + k)));
        float2 a = float2(0.0, at);
        float2 b = a + (float2(lean, 0.55 + (0.3 * Hash1(seed + k + 7.0))) * h * (0.42 - (0.06 * k)));
        d = min(d, Segment(p, a, b) - (h * (0.020 - (0.003 * k))));
        float2 c = b + (float2(lean * 0.6, 0.5) * h * 0.16);
        float2 c2 = b + (float2(-lean * 0.3, 0.7) * h * 0.14);
        d = min(d, Segment(p, b, c) - (h * 0.008));
        d = min(d, Segment(p, b, c2) - (h * 0.007));
    }
    return d;
}

// One layer of bats: a few to a ring of the sky, each beating its jagged wings at its own pace.
// Returns how much of the point a bat covers.
float Bats(float phi, float e, float pixel, float clock, float band, float chance, float layer)
{
    float row = floor(e / band);
    float ring = (row + 0.5) * band;
    float slots = max(1.0, floor((TAU * cos(clamp(ring, -1.55, 1.55))) / band));
    float slot_width = TAU / slots;
    float at = (phi + PI) / slot_width;
    float3 id = float3(fmod(floor(at), slots), row, layer * 13.0);
    float2 local = float2(frac(at), frac(e / band));
    float2 extent = float2(slot_width * max(cos(e), 0.05), band);
    float span = band * (0.10 + (0.08 * Hash3(id + 3.3)));
    float2 centre = 0.3 + (0.4 * float2(Hash3(id + 7.1), Hash3(id + 13.7)));
    float beat = sin((Ease(clock) * TAU * (4.0 + (3.0 * Hash3(id + 5.5)))) + (Hash3(id + 9.9) * TAU));
    float2 d = ((local - centre) * extent) / span;
    d.y -= 0.10 * beat;
    float2 w = float2(abs(d.x), d.y);
    // The wing: its leading edge rising to the tip with the beat, its trailing edge scalloped.
    float lift = 0.45 * beat;
    float top = (0.25 * w.x) + (lift * w.x * w.x);
    float scallop = top - 0.18 - (0.10 * abs(sin(w.x * PI * 3.0))) + (0.08 * w.x);
    float wing = max(max(w.y - top, scallop - w.y), w.x - 1.0);
    // The body, and two pointed ears on top of the head.
    float body = (length(d / float2(0.10, 0.16)) - 1.0) * 0.10;
    float ears = max(abs(w.x - 0.05) - (0.035 * saturate((0.26 - w.y) / 0.12)), max(0.10 - w.y, w.y - 0.26));
    float shape = min(min(wing, body), ears);
    return Fill(shape * span, pixel) * step(Hash3(id + 21.3), chance);
}
