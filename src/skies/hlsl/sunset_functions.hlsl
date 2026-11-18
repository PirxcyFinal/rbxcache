// Sunset Ocean: the sun low over the sea, the sky warm round it, clouds lit pink and orange from
// beneath, the sea glittering in a path of light toward the sun, and a few gulls flapping.

static const float TAU = 6.2831853;
static const float PI = 3.14159265;

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

// Where the sun is: low over the horizon, on the bearing of the game's own sun so the shadows
// in the game lean the right way, or toward the south-west if the game gives no sun.
float3 SunDirection()
{
    float3 lamp = -G[16].xyz;
    float bearing = (dot(lamp.xz, lamp.xz) > 1e-6) ? atan2(lamp.z, lamp.x) : 0.6;
    float rise = 0.065;
    return float3(cos(bearing) * cos(rise), sin(rise), sin(bearing) * cos(rise));
}

// The colour of the open sky in direction `d`, with the sun toward `sun`, before clouds.
float3 SkyColour(float3 d, float3 sun)
{
    float up = saturate(d.y);
    float toward = saturate(dot(normalize(float3(d.x, 0.0, d.z) + 1e-5), normalize(float3(sun.x, 0.0, sun.z))));
    float3 horizon = lerp(float3(0.55, 0.30, 0.48), float3(1.00, 0.52, 0.20), pow(toward, 3.0));
    float3 sky = lerp(horizon, lerp(float3(0.42, 0.30, 0.52), float3(0.85, 0.45, 0.32), pow(toward, 4.0)), smoothstep(0.0, 0.18, up));
    sky = lerp(sky, float3(0.12, 0.16, 0.36), smoothstep(0.15, 0.85, up));
    // The glow round the sun.
    float angle = acos(clamp(dot(d, sun), -1.0, 1.0));
    sky += float3(1.0, 0.55, 0.22) * 0.55 * exp(-angle / 0.16);
    sky += float3(1.0, 0.78, 0.45) * 0.45 * exp(-angle / 0.045);
    return sky;
}

// One layer of gulls: a few to a ring of the sky, each a pair of bent wings beating at its own
// pace. Returns how much of the point a gull covers.
float Gulls(float phi, float e, float pixel, float clock, float band, float chance)
{
    float row = floor(e / band);
    float ring = (row + 0.5) * band;
    float slots = max(1.0, floor((TAU * cos(clamp(ring, -1.55, 1.55))) / band));
    float slot_width = TAU / slots;
    float at = (phi + PI) / slot_width;
    float3 id = float3(fmod(floor(at), slots), row, 5.0);
    float2 local = float2(frac(at), frac(e / band));
    float2 extent = float2(slot_width * max(cos(e), 0.05), band);
    float span = 0.010 + (0.010 * Hash3(id + 3.3));
    float2 centre = 0.3 + (0.4 * float2(Hash3(id + 7.1), Hash3(id + 13.7)));
    float beat = sin((Ease(clock) * TAU * (3.0 + (2.0 * Hash3(id + 5.5)))) + (Hash3(id + 9.9) * TAU));
    float2 d = ((local - centre) * extent) / span;
    centre.y += 0.0;
    d.y -= beat * 0.08;
    // Each wing an arc from the body out to its tip, the tips rising and falling with the beat.
    float2 w = float2(abs(d.x), d.y);
    float lift = 0.35 * beat;
    float curve = w.y - ((0.45 * w.x) * (1.0 - w.x) * 1.8) - (lift * w.x * w.x);
    float wing = max(abs(curve) - (0.07 * (1.0 - (0.6 * w.x))), w.x - 1.0);
    return Fill(wing * span, pixel) * step(Hash3(id + 21.3), chance);
}
