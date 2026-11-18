// Shooting Stars: a clear blue night. Everything is fixed in the sky, so it stays put as the
// camera turns.

static const float TAU = 6.2831853;
static const float PI = 3.14159265;

float Hash1(float x) { return frac(sin(x * 127.1) * 43758.5453); }
float Hash3(float3 p) { return frac(sin(dot(p, float3(127.1, 311.7, 74.7))) * 43758.5453); }

// Smooth noise in three dimensions, 0 to 1, so it lies on the sky without a seam.
float Noise3(float3 p)
{
    float3 cell = floor(p);
    float3 f = frac(p);
    f = f * f * (3.0 - (2.0 * f));
    float a = lerp(Hash3(cell), Hash3(cell + float3(1.0, 0.0, 0.0)), f.x);
    float b = lerp(Hash3(cell + float3(0.0, 1.0, 0.0)), Hash3(cell + float3(1.0, 1.0, 0.0)), f.x);
    float c = lerp(Hash3(cell + float3(0.0, 0.0, 1.0)), Hash3(cell + float3(1.0, 0.0, 1.0)), f.x);
    float d = lerp(Hash3(cell + float3(0.0, 1.0, 1.0)), Hash3(cell + float3(1.0, 1.0, 1.0)), f.x);
    return lerp(lerp(a, b, f.y), lerp(c, d, f.y), f.z);
}

// Layers of noise, 0 to 1.
float Cloud3(float3 p)
{
    float sum = 0.0;
    float weight = 0.5;
    for (int octave = 0; octave < 4; octave++)
    {
        sum += weight * Noise3(p);
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

// One layer of stars. The sky is cut into rings `band` radians high, each cut into as many cells
// as fit round it so the cells are near square everywhere, and a share `chance` of the cells
// hold a star somewhere in their middle.
//   phi, e   the direction: round the horizon and up, in radians
//   pixel    a screen pixel in radians
//   bright   how bright the brightest of the layer are
float3 Stars(float phi, float e, float pixel, float clock, float band, float chance, float bright, float layer)
{
    float row = floor(e / band);
    float ring = (row + 0.5) * band;
    float slots = max(1.0, floor((TAU * cos(clamp(ring, -1.55, 1.55))) / band));
    float slot_width = TAU / slots;
    float at = (phi + PI) / slot_width;
    float3 id = float3(fmod(floor(at), slots), row, layer * 37.0);
    float2 local = float2(frac(at), frac(e / band));
    // How big the cell is, in radians, across at this height and up.
    float2 extent = float2(slot_width * max(cos(e), 0.02), band);
    float2 centre = 0.3 + (0.4 * float2(Hash3(id), Hash3(id + 7.1)));
    float r = length((local - centre) * extent);
    float size = Hash3(id + 3.3);
    // A star is drawn about a pixel across however small it is, so faint ones stay smooth and
    // do not sparkle as the view turns. The brightest few carry a soft glow, kept inside the cell.
    float core = exp(-(r * r) / (pixel * pixel));
    float glow = pow(size, 8.0) * 0.30 * exp(-r / (pixel * 2.5)) * saturate(1.0 - (r / (0.30 * min(extent.x, extent.y))));
    float light = ((0.10 + (0.90 * pow(size, 5.0))) * core) + glow;
    // Each twinkles slowly at its own pace on the eased clock.
    float twinkle = 0.70 + (0.30 * cos((Hash3(id + 9.9) * TAU) + (TAU * Ease(clock) * (1.0 + (2.0 * Hash3(id + 5.5))))));
    float t = Hash3(id + 12.7);
    float3 tint = (t < 0.12) ? float3(1.00, 0.82, 0.62) : ((t < 0.42) ? float3(0.72, 0.84, 1.00) : float3(0.93, 0.96, 1.00));
    return step(Hash3(id + 21.3), chance) * light * bright * twinkle * tint;
}

// Shooting star `k` of `count`. Each has its own path across the sky and its own stretch of the
// clock, RUN long, and flies along the path as the clock runs through the first part of that
// stretch, burning out by BURN of the way. The clock runs back and forth, so on the way back
// down it flies the path the other way, fading in and ending where it began, which a shooting
// star also does. Burning out early means that when a swing of the clock stops inside the
// stretch, it mostly stops where the star is no longer seen: on the real clock only about one
// run in ten visibly turns back. The star is a bright head in a streak of its own light that
// fades the same way on both sides, so it looks right whichever way it flies.
static const float RUN = 0.20;
static const float BURN = 0.45;

float3 Meteor(float3 n, float pixel, float clock, float k, float count)
{
    float c0 = 0.60 + (0.20 * (k + 0.25 + (0.5 * Hash1((k * 13.1) + 8.8))) / count);
    float s = (clock - c0) / RUN;
    float bright = smoothstep(0.0, 0.06, s) * (1.0 - smoothstep(BURN * 0.6, BURN, s));
    [branch]
    if (bright <= 0.0)
    {
        return float3(0.0, 0.0, 0.0);
    }
    // Where it starts: well up the sky, each well round the horizon from the last.
    float phi = frac((k * 0.618034) + 0.13 + (0.12 * Hash1((k * 3.7) + 1.3))) * TAU;
    float el = 0.40 + (0.55 * Hash1((k * 5.1) + 2.9));
    float3 at = float3(cos(el) * cos(phi), sin(el), cos(el) * sin(phi));
    // Which way it goes: across the sky and a little down, to the left or the right, along a
    // great circle.
    float3 across = float3(-sin(phi), 0.0, cos(phi));
    float3 upward = cross(across, at);
    upward *= sign(upward.y);
    float slant = radians(8.0 + (24.0 * Hash1((k * 9.7) + 0.5)));
    float3 way = (across * ((Hash1((k * 7.3) + 4.1) < 0.5) ? -1.0 : 1.0) * cos(slant)) - (upward * sin(slant));
    float3 aside = cross(at, way);
    float span = 0.20 + (0.10 * Hash1((k * 11.3) + 6.6));
    // The point against the path: how far along it from the start, and how far to one side, in
    // radians; and where the head has got to.
    float theta = atan2(dot(n, way), dot(n, at));
    float off = dot(n, aside);
    float along = theta - (span * saturate(s / BURN));
    // A streak about two degrees long, a pixel or so wide, round a bright head.
    float streak = exp(-(along * along) / (0.035 * 0.035)) * exp(-(off * off) / (pixel * pixel * 1.2));
    float head = exp(-((along * along) + (off * off)) / (pixel * pixel * 4.0));
    float3 colour = (streak * float3(0.55, 0.78, 1.00) * 0.85) + (head * float3(1.0, 1.0, 1.0));
    return colour * bright * step(0.0, dot(n, at));
}
