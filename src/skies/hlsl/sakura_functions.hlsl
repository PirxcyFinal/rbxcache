static const float TAU = 6.2831853;

float Hash1(float x) { return frac(sin(x * 127.1) * 43758.5453); }
float Hash2(float2 p) { return frac(sin(dot(p, float2(127.1, 311.7))) * 43758.5453); }
float Hash3(float3 p) { return frac(sin(dot(p, float3(127.1, 311.7, 74.7))) * 43758.5453); }

float Noise2(float2 p)
{
    float2 cell = floor(p);
    float2 f = frac(p);
    f = f * f * (3.0 - 2.0 * f);
    return lerp(lerp(Hash2(cell), Hash2(cell + float2(1.0, 0.0)), f.x),
                lerp(Hash2(cell + float2(0.0, 1.0)), Hash2(cell + float2(1.0, 1.0)), f.x), f.y);
}

float Cloud(float2 p)
{
    float sum = 0.0;
    float weight = 0.5;
    for (int octave = 0; octave < 4; octave++)
    {
        sum += weight * Noise2(p);
        p = (p * 2.07) + 17.13;
        weight *= 0.5;
    }
    return sum;
}

// The snow's pace off the sky clock, which runs back and forth: it keeps petals changing
// while the clock sits near zero, and never races.
float Pace(float clock)
{
    float c = min(saturate(clock), 0.999);
    return (0.65 * asin(c)) + (0.175 * log((1.0 + c) / (1.0 - c))) + (0.1 * log(c + 0.06));
}

// How far `p` is from the segment `a` to `b`, which is `ra` thick at `a` and `rb` at `b`.
float Limb(float2 p, float2 a, float2 b, float ra, float rb)
{
    float2 pa = p - a;
    float2 ba = b - a;
    float h = saturate(dot(pa, ba) / dot(ba, ba));
    return length(pa - (ba * h)) - lerp(ra, rb, h);
}

// The smaller of two distances, rounded where they meet, so the blossom reads as one cloud.
float Merge(float a, float b, float k)
{
    float h = saturate(0.5 + (0.5 * (b - a) / k));
    return lerp(b, a, h) - (k * h * (1.0 - h));
}

// One cherry tree, in its own units: it stands at the origin, about one unit to the fork and
// two to the top of the blossom, spreading a little over a unit to each side. `seed` shapes
// it, `sway` moves the blossom a touch in the wind, and `pixel` is how big a screen pixel is
// in the tree's units, for smooth edges. Returns the tree's colour in .rgb and how much of the
// pixel it covers in .a.
float4 Tree(float2 p, float seed, float sway, float pixel)
{
    // Some lean the other way.
    p.x *= (Hash1(seed + 0.3) < 0.5) ? -1.0 : 1.0;
    float lean = (Hash1(seed + 1.7) - 0.5) * 0.24;
    float2 fork = float2(lean, 0.95);
    float wood = Limb(p, float2(0.0, -0.4), fork, 0.10, 0.065);
    wood = min(wood, Limb(p, fork, fork + float2(-0.72, 0.42), 0.055, 0.022));
    wood = min(wood, Limb(p, fork, fork + float2(0.70, 0.36), 0.052, 0.022));
    wood = min(wood, Limb(p, fork + float2(0.02, -0.12), fork + float2(0.20, 0.66), 0.045, 0.02));
    wood = min(wood, Limb(p, fork + float2(-0.40, 0.23), fork + float2(-0.95, 0.30), 0.028, 0.012));
    wood = min(wood, Limb(p, fork + float2(0.40, 0.21), fork + float2(0.98, 0.20), 0.026, 0.012));

    // The blossom: a broad dome of round clusters, each tree its own.
    float2 crown = fork + float2(0.0, 0.42);
    float bloom = 1e5;
    for (int k = 0; k < 8; k++)
    {
        float a = (k / 8.0) * TAU + (Hash1(seed + k) * 0.6);
        float2 at = crown + float2(cos(a) * 0.72, sin(a) * 0.36) + float2(0.0, 0.05 * sin(a * 2.0 + seed));
        float r = 0.30 + (0.16 * Hash1(seed + (k * 3.1) + 5.0));
        bloom = Merge(bloom, length(p - at) - r, 0.18);
    }
    bloom = Merge(bloom, length(p - crown) - 0.52, 0.18);
    // A soft, broken edge of flowers, and a few gaps the branches show through.
    float2 q = p + float2(sway * 0.05, 0.0);
    float lump = (Noise2((q * 5.0) + seed * 7.0) - 0.5) * 0.30 + (Noise2((q * 13.0) + seed * 3.0) - 0.5) * 0.12;
    bloom += lump;

    // The edges are a pixel wide. The width comes from the view, not from how the distance
    // changes between pixels: next to a pixel of another tree that change is a jump.
    float edge = max(pixel * 0.75, 0.002);
    float has_wood = 1.0 - smoothstep(-edge, edge, wood);
    float has_bloom = 1.0 - smoothstep(-edge, edge, bloom);

    // Lit from above: pale on top, deeper pink underneath, with single flowers catching the light.
    float height = saturate((p.y - fork.y) / 1.0);
    float3 pink = lerp(float3(0.80, 0.46, 0.60), float3(1.00, 0.84, 0.90), smoothstep(0.0, 0.9, height));
    float flowers = Noise2((q * 30.0) + seed * 11.0);
    pink = lerp(pink, float3(1.00, 0.92, 0.95), smoothstep(0.68, 0.95, flowers) * 0.40);
    pink *= 0.92 + (0.16 * Noise2((q * 3.0) + seed));
    // Deeper inside the cloud of blossom, a little darker.
    pink *= 1.0 - (0.18 * saturate(-bloom * 3.0));
    // Darker toward the ground.
    float3 bark = float3(0.20, 0.13, 0.13) * (0.80 + (0.25 * saturate(p.y)));

    // Where the blossom's edge is soft over the sky, the colour is still all blossom.
    float cover = max(has_bloom, has_wood);
    float3 colour = lerp(bark, pink, saturate(has_bloom / max(cover, 0.0001)));
    return float4(colour, cover);
}

// A row of trees round the horizon, `cells` places to a full turn, most with a tree. `u` is
// how far round (0 to 1) and `e` the elevation in radians; `base` is where the trees stand
// and `size` how big one unit of a tree is, both in radians; `pixel` is a screen pixel in radians.
float4 Grove(float u, float e, float cells, float base, float size, float row, float sway, float pixel)
{
    float4 result = float4(0.0, 0.0, 0.0, 0.0);
    float at = u * cells;
    float here = floor(at);
    [unroll]
    for (int k = -1; k <= 1; k++)
    {
        float place = here + k;
        // Each tree belongs to its place round the circle, whichever side of the seam it is seen from.
        float id = fmod(fmod(place, cells) + cells, cells) + (row * 101.0);
        float scale = size * (0.80 + (0.40 * Hash1(id + 2.3)));
        float centre = place + 0.5 + ((Hash1(id + 4.1) - 0.5) * 0.5);
        float2 p = float2(((at - centre) / cells) * TAU, e - base) / scale;
        // Only a place with a tree, and only inside that tree's reach, is worth the work.
        // Nothing in a tree looks at the pixels beside it, so skipping one is safe.
        float4 tree = float4(0.0, 0.0, 0.0, 0.0);
        [branch]
        if (Hash1(id + 0.9) <= 0.82 && abs(p.x) < 1.9 && p.y > -0.5 && p.y < 2.6)
        {
            tree = Tree(p, id * 1.37, sway, pixel / scale);
        }
        float cover = tree.a;
        // Laid over what is there already, colour weighted by how much it covers.
        result.rgb = (result.rgb * (1.0 - cover)) + (tree.rgb * cover);
        result.a = (result.a * (1.0 - cover)) + cover;
    }
    // Back to the colour itself, so a soft edge is the blossom's colour, only fainter.
    return float4(result.rgb / max(result.a, 0.0001), result.a);
}

// How far the petals have glided, 0 to 1, for the sky clock `clock`. It eases to a stop at both
// ends: at the top of a swell the clock turns smoothly anyway, but where it touches zero it
// turns at once, and without the ease the petals would jolt there.
float Glide(float clock)
{
    float c = saturate(clock);
    return c * c * (3.0 - (2.0 * c));
}

// One layer of cherry petals gliding down the sky, fixed in the sky like the aurora's stars so
// turning the camera sweeps past them. The sky is cut into rings of elevation `band` radians
// tall, and each ring into as many cells as fit round it at that height, so cells are much the
// same size everywhere. A petal lives in a cell, with room to spare, so none is cut off at a
// cell's edge or at the seam behind the viewer.
//
// The whole layer glides `glide` radians down the sky as the sky clock runs up, and glides back
// up as it runs down: the clock runs back and forth and the shader cannot tell which way, so
// the petals go with it, slowing to a stop at each turn and setting off again the other way.
// Nearer layers glide farther. As it comes down each petal sways from side to side like a
// falling leaf, along a shallow arc that lifts a little at each end, tilting into the swing,
// and in time fades out as another fades in somewhere else. All of it runs off the same eased
// glide, so it all turns round together, smoothly.
//   phi     how far round the horizon, in radians (-pi to pi)
//   e       the elevation, in radians
//   pixel   one screen pixel, in radians
//   rate    showings per run of the clock
//   size    a petal's half length, in radians
// Returns how much petal covers the pixel in .a, and how far toward the petal's paler middle in .r.
float2 PetalSky(float phi, float e, float pixel, float clock, float sheet, float band, float glide, float rate, float size)
{
    // The layer slides down, so the place on it that shows here is higher up it.
    float field = e + (glide * Glide(clock));
    float row = floor(field / band);
    float ring = (row + 0.5) * band;
    float slots = max(1.0, floor((TAU * cos(clamp(ring, -1.55, 1.55))) / band));
    float slot_width = TAU / slots;
    float at = (phi + 3.14159265) / slot_width;
    float3 id = float3(fmod(floor(at), slots), row, sheet * 91.0);
    // Where the pixel is in its cell, 0 to 1 across and down the sky.
    float2 local = float2(frac(at), 1.0 - frac(field / band));
    // How big the cell is, in radians, across at this height and down.
    float2 extent = float2(slot_width * max(cos(e), 0.02), band);
    // Neighbouring cells run at quite different speeds, so the petals never all fade together.
    float speed = 0.5 + Hash3(id);
    float phase = Hash3(id + 0.37);
    float eased = Glide(clock);
    float drive = rate * eased;
    float cover = 0.0;
    float middle = 0.0;
    for (int turn = 0; turn < 4; turn++)
    {
        // Four generations a quarter beat apart. A petal spends half its life easing in and the
        // other half easing out, so nothing appears or goes quickly; one generation eases out
        // exactly as another eases in, so the petals never pulse.
        float beat = (drive * speed) + phase + (turn * 0.25);
        float age = frac(beat);
        float fade = smoothstep(0.0, 0.5, age) * (1.0 - smoothstep(0.5, 1.0, age));
        float3 showing = id + float3(floor(beat) * 13.7, turn * 3.1, 1.0);
        float length_ = max(size * (0.75 + (0.5 * Hash3(showing + 6.6))), pixel * 1.6);
        // The petal's half length in cells, across and down.
        float2 w = length_ / extent;
        // The petal and the room it sways in stay inside its cell.
        float2 room = min(float2(w.x * 3.6, w.y * 2.2), 0.45);
        float2 centre = room + ((1.0 - (2.0 * room)) * float2(Hash3(showing + 2.2), Hash3(showing + 5.1)));
        // The falling-leaf sway: side to side, lifting a little at each end of the swing, slow
        // enough that it drifts sideways about as fast as it comes down. Each petal is at its
        // own point in the swing.
        float rock = (TAU * SWAYS * eased * (0.8 + (0.4 * Hash3(showing + 7.3)))) + (Hash3(showing + 1.9) * TAU);
        float swing = sin(rock);
        centre.x += w.x * 1.8 * swing;
        centre.y -= w.y * 0.7 * swing * swing;
        float2 d = (local - centre) / w;
        // It tilts into the swing, most as it passes the middle where it moves fastest, and
        // turns over slowly; seen more edge on it is thinner.
        float tilt = (Hash3(showing + 4.4) * TAU) + (0.7 * cos(rock));
        float2 r = float2((d.x * cos(tilt)) + (d.y * sin(tilt)), (-d.x * sin(tilt)) + (d.y * cos(tilt)));
        float flip = 0.45 + (0.55 * abs(cos((rock * 0.5) + (Hash3(showing + 8.8) * TAU))));
        r.x /= flip * 0.62;
        // A rounded petal, narrower at its base, with the little notch of a cherry petal at its tip.
        float body = length(float2(r.x * (1.0 + (0.35 * saturate(-r.y))), r.y)) - 1.0;
        float notch = length(r - float2(0.0, 1.08)) - 0.30;
        float petal = max(body, -notch);
        // A pixel wide at the edge, worked out from the petal's size rather than from its
        // neighbours, which may belong to another petal.
        float soft = max((pixel * 1.2) / (length_ * flip * 0.62), 0.08);
        float show = (1.0 - smoothstep(-soft, soft, petal)) * fade * step(Hash3(showing + 8.4), PETALS);
        cover = max(cover, show);
        middle = max(middle, show * saturate(1.0 - length(r)));
    }
    return float2(middle, cover);
}

// The canopy overhead. Looking up, the view is cut onto a flat roof of blossom one unit above
// (see the effect), so branches and flowers farther off look smaller and thinner, as they would.

float2 Turn(float a) { return float2(cos(a), sin(a)); }

// How far `p` is from the stretch `a` to `b`, and how far along it the nearest point is, 0 to 1.
float2 Along(float2 p, float2 a, float2 b)
{
    float2 pa = p - a;
    float2 ba = b - a;
    float h = saturate(dot(pa, ba) / dot(ba, ba));
    return float2(length(pa - (ba * h)), h);
}

// A branch from `a` to `b`, `ra` thick at `a` and `rb` at `b`: how far `p` is from its bark
// (below zero inside it), and from the line down its middle.
float2 Bough(float2 p, float2 a, float2 b, float ra, float rb)
{
    float2 s = Along(p, a, b);
    return float2(s.x - lerp(ra, rb, s.y), s.x);
}

// Where the trunk forks, off to one side of straight up, and the way the trunk comes in from.
static const float2 FORK = float2(-0.9, -0.55);
static const float TRUNK_WAY = 3.75;

// The trunk, from the horizon up to the fork: how far `c` is from its bark. It is near, so it
// keeps much the same width on screen all the way down, which on the roof means it widens in
// step with how far out it is.
float Trunk(float2 c)
{
    float2 from = FORK + (Turn(TRUNK_WAY) * 400.0);
    float2 s = Along(c, from, FORK);
    float2 at = lerp(from, FORK, s.y);
    return s.x - (0.09 + (0.055 * length(at)));
}

// The trunk low down, where it stands beside the viewer: an upright band `TRUNK_WIDTH` radians
// wide, at the bearing the trunk comes in from. `u` is how far round the horizon (0 to 1).
// Returns how far off the band the pixel is, in radians.
static const float TRUNK_WIDTH = 0.055;
float TrunkLow(float u)
{
    float off = (u * TAU) - TRUNK_WAY;
    off -= TAU * floor((off / TAU) + 0.5);
    return abs(off) - TRUNK_WIDTH;
}

// The tree overhead: seven crooked limbs fanning out from the top of the trunk, four smaller
// branches on each, and two twigs on each of those. Returns how far `c` is from the nearest
// bark in .x, and in .y from where flowers crowd: twigs and the ends of the smaller branches.
// Limbs count as much farther, so they stay dark through the blossom.
float2 Canopy(float2 c)
{
    // A slow wobble through the whole tree, so no branch runs quite straight.
    c += (float2(Noise2(c * 1.6), Noise2((c * 1.6) + 5.3)) - 0.5) * 0.16;
    float wood = 1e5;
    float twigs = 1e5;
    [loop]
    for (int i = 0; i < 7; i++)
    {
        float h1 = Hash1((i * 7.3) + 1.0);
        float h2 = Hash1((i * 5.1) + 2.0);
        float h3 = Hash1((i * 3.7) + 3.0);
        // Fanned out away from the trunk, the outer ones curling back round its sides.
        float a = TRUNK_WAY + 3.14159 + (((i / 6.0) - 0.5) * 3.9) + ((h1 - 0.5) * 0.3);
        float reach = 4.0 + (2.5 * h2);
        // Three crooked lengths, each turned a little from the last.
        float a1 = a + ((h3 - 0.5) * 0.6);
        float a2 = a1 + ((Hash1((i * 2.9) + 4.0) - 0.5) * 0.9);
        float a3 = a2 + ((Hash1((i * 4.3) + 5.0) - 0.5) * 0.9);
        float2 p1 = FORK + (Turn(a1) * (reach * 0.30));
        float2 p2 = p1 + (Turn(a2) * (reach * 0.35));
        float2 p3 = p2 + (Turn(a3) * (reach * 0.35));
        float2 s1 = Bough(c, FORK, p1, 0.085, 0.060);
        float2 s2 = Bough(c, p1, p2, 0.060, 0.034);
        float2 s3 = Bough(c, p2, p3, 0.034, 0.012);
        wood = min(wood, min(s1.x, min(s2.x, s3.x)));
        twigs = min(twigs, min(s1.y, min(s2.y, s3.y)) + 0.40);
        // Its smaller branches only matter near it, so far from it they are not worked out.
        [branch]
        if (min(Along(c, FORK, p2).x, Along(c, p2, p3).x) < 2.6)
        {
            for (int j = 0; j < 4; j++)
            {
                float f = 0.22 + (j * 0.22) + ((Hash1((i * 11.0) + j) - 0.5) * 0.08);
                float2 root = (f < 0.30) ? lerp(FORK, p1, f / 0.30) : ((f < 0.65) ? lerp(p1, p2, (f - 0.30) / 0.35) : lerp(p2, p3, (f - 0.65) / 0.35));
                float side = ((j % 2) == 0) ? 1.0 : -1.0;
                float way = a2 + (side * (0.50 + (0.50 * Hash1((i * 13.0) + (j * 1.7)))));
                float length_ = (1.0 + (0.8 * Hash1((i * 17.0) + (j * 2.3)))) * (1.15 - (f * 0.5));
                float2 knee = root + (Turn(way) * (length_ * 0.5));
                float2 tip = knee + (Turn(way + (side * (Hash1((i * 29.0) + j) - 0.3) * 0.8)) * (length_ * 0.5));
                float2 s4 = Bough(c, root, knee, 0.030, 0.018);
                float2 s5 = Bough(c, knee, tip, 0.018, 0.006);
                wood = min(wood, min(s4.x, s5.x));
                twigs = min(twigs, min(s4.y + 0.12, s5.y));
                for (int k = 0; k < 2; k++)
                {
                    float2 from = (k == 0) ? knee : lerp(knee, tip, 0.6);
                    float bend = way + ((k == 0) ? -side : side) * (0.7 + (0.4 * Hash1((i * 19.0) + (j * 3.1) + k)));
                    float2 to = from + (Turn(bend) * (0.40 + (0.35 * Hash1((i * 23.0) + (j * 4.3) + k))));
                    float2 s6 = Bough(c, from, to, 0.010, 0.003);
                    wood = min(wood, s6.x);
                    twigs = min(twigs, s6.y);
                }
            }
        }
    }
    return float2(wood, twigs);
}

// Cherry flowers on the roof: it is cut into cells, `cells` to a unit, and a flower sits in
// a cell when `density` there is high enough for it. Five rounded petals, paler at their tips,
// round a deeper pink centre. `pixel` is a screen pixel in roof units, for smooth edges. Where
// a flower would be too small to draw, it fades out and leaves the soft blossom behind it.
// Returns the colour in .rgb and how much the flowers cover in .a.
float4 Flowers(float2 c, float cells, float layer, float pixel, float density)
{
    float2 g = c * cells;
    float2 first = floor(g - 0.5);
    float edge = max(pixel * cells, 0.02);
    float4 result = float4(0.0, 0.0, 0.0, 0.0);
    [unroll]
    for (int k = 0; k < 4; k++)
    {
        float2 cell = first + float2(k % 2, k / 2);
        float3 id = float3(cell, layer * 37.0);
        float2 centre = cell + 0.5 + ((float2(Hash3(id + 1.3), Hash3(id + 2.9)) - 0.5) * 0.5);
        float2 d = g - centre;
        float r = length(d);
        float size = 0.45 + (0.20 * Hash3(id + 6.1));
        float a = atan2(d.y, d.x) + (Hash3(id + 4.7) * TAU);
        float outline = size * (0.70 + (0.30 * abs(cos(2.5 * a))));
        float shape = 1.0 - smoothstep(-edge, edge, r - outline);
        float wanted = Hash3(id);
        // A flower is there or it is not: a faint one over dark bark reads as a stain.
        float present = smoothstep(wanted - 0.03, wanted + 0.03, density);
        float3 pink = lerp(float3(0.99, 0.80, 0.88), float3(0.94, 0.56, 0.71), Hash3(id + 8.3));
        pink = lerp(pink, float3(1.0, 0.93, 0.96), 0.35 * smoothstep(0.25, 0.95, r / size));
        pink = lerp(float3(0.86, 0.42, 0.60), pink, smoothstep(0.06, 0.22, r / size));
        float cover = shape * present;
        result.rgb = (result.rgb * (1.0 - cover)) + (pink * cover);
        result.a = (result.a * (1.0 - cover)) + cover;
    }
    // A flower a few pixels across is just a speck; it fades into the soft blossom instead.
    float shown = smoothstep(3.0, 6.0, 1.0 / max(pixel * cells, 0.0001));
    return float4(result.rgb / max(result.a, 0.0001), result.a * shown);
}
