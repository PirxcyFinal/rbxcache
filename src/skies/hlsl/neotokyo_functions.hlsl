// Neo Tokyo: a city at night all round, close by, in a light rain. These follow the rain's
// functions and the glyph table in the same shader, so Hash1, Hash2, Hash3, Noise2, Cloud,
// Downpour, GLYPH_DATA, WORDS and WORD_TEXT come from there.
//
// Two rings of buildings stand round the viewer, one close and one a street behind, drawn as
// facades in perspective: a point on a facade `D` away and `h` above the eye shows at elevation
// atan(h / D), so the floors crowd together toward the tops, as they do looking up at a real
// building. Behind them, low rows of distant buildings run along the horizon.

static const float TAU = 6.2831853;
static const float PI = 3.14159265;
// How high the eye is above the street, in the units the buildings are measured in (the
// nearest ring stands 0.7 of them away).
static const float EYE = 0.05;
// The ground floor's height, the shops' floor, in the same units.
static const float GROUND = 0.085;

// The sky clock eased to a stop at both ends, so nothing driven by it jerks where it turns at zero.
float Ease(float clock)
{
    float c = saturate(clock);
    return c * c * (3.0 - (2.0 * c));
}

// Neon colours: pink, green, cyan, violet, amber, red. The eye takes green and cyan for far
// brighter than pink or red at the same strength, and in game the bloom made them glare, so
// those are held down to look about as bright as the rest.
float3 Neon(float pick)
{
    float k = floor(pick * 6.0);
    return (k < 1.0) ? float3(1.00, 0.26, 0.62)
        : (k < 2.0) ? float3(0.28, 0.76, 0.42)
        : (k < 3.0) ? float3(0.20, 0.70, 0.92)
        : (k < 4.0) ? float3(0.66, 0.38, 1.00)
        : (k < 5.0) ? float3(0.95, 0.58, 0.20)
        : float3(1.00, 0.24, 0.28);
}

// How much of a point a soft-edged shape covers: `d` is how far inside the shape the point is,
// below zero outside, and `edge` is how wide the soft edge is.
float Inside(float d, float edge)
{
    return smoothstep(-edge, edge, d);
}

// Integer hashing. The sine hash the far rows use clumps for large numbers, and the near
// buildings have many, so their choices come from here.
uint Mix(uint x)
{
    x ^= x >> 16;
    x *= 0x7feb352du;
    x ^= x >> 15;
    x *= 0x846ca68bu;
    x ^= x >> 16;
    return x;
}

// A number from 0 to 1 for one of a thing's choices; `salt` says which.
float Pick(float id, uint salt)
{
    return (float)(Mix(((uint)id * 747796405u) + (salt * 2891336453u)) & 0xffffffu) / 16777216.0;
}

// The same for one window, floor or other cell of a building.
float Pick3(float2 cell, float id, uint salt)
{
    uint x = Mix(((uint)((int)id) * 747796405u) + (salt * 2891336453u));
    x = Mix(x ^ ((uint)((int)cell.x) * 2246822519u));
    x = Mix(x ^ ((uint)((int)cell.y) * 3266489917u));
    return (float)(x & 0xffffffu) / 16777216.0;
}

// A box (left, right, bottom, top). One with its bottom above its top holds nothing.
static const float4 NO_BOX = float4(0.0, 0.0, 1.0, -1.0);

// How much of box `box` covers point `p`, with an edge a screen pixel wide: `aa` is how long a
// screen pixel is across and up, in the same units as `p`.
float BoxCover(float2 p, float4 box, float2 aa)
{
    float2 d = min(p - box.xz, box.yw - p) / aa;
    return saturate(0.5 + min(d.x, d.y));
}

// How deep inside a box a point is; below zero outside.
float BoxDepth(float2 p, float4 box)
{
    float2 d = min(p - box.xz, box.yw - p);
    return min(d.x, d.y);
}

// How far outside a box a point is; 0 inside.
float BoxOutside(float2 p, float4 box)
{
    return length(max(max(box.xz - p, p - box.yw), 0.0));
}

// How brightly a sign burns. Most burn steadily; now and then one buzzes for a beat, dropping
// out for a moment at a time the way a failing neon tube does.
float Hum(float id, float clock)
{
    float beat = (Ease(clock) * 4.0) + Pick(id, 155u);
    float buzzing = step(Pick(id + (floor(beat) * 977.0), 33u), 0.16);
    float stutter = step(Pick(id + (floor(beat * 5.0) * 131.0), 44u), 0.45);
    return 1.0 - (buzzing * stutter * 0.75);
}

// The red light on a mast, blinking slowly: 0 to 1.
float Beacon(float id, float clock)
{
    float beat = frac((Ease(clock) * 2.0) + Pick(id, 202u));
    return smoothstep(0.30, 0.42, beat) * (1.0 - smoothstep(0.62, 0.80, beat));
}

// A light that now and then goes on or off; `lit` is how it starts. It keeps its own beat on
// the eased clock and on a few beats it changes, crossfading over the end of the beat, so the
// city's lights come and go softly rather than blinking. `id` should be a whole number.
float Switching(float id, float clock, float lit)
{
    float beat = (Ease(clock) * 3.0 * (0.6 + (0.8 * Pick(id, 55u)))) + Pick(id, 66u);
    float here = floor(beat);
    float a = (Pick(id + (here * 7.0), 77u) < 0.07) ? (1.0 - lit) : lit;
    float b = (Pick(id + ((here + 1.0) * 7.0), 77u) < 0.07) ? (1.0 - lit) : lit;
    return lerp(a, b, smoothstep(0.65, 1.0, frac(beat)));
}

// ---------------------------------------------------------------------------------------------
// The distant rows along the horizon. Here `x` is how far across its place a point is, as a
// share of it, and heights are elevations in radians.

// Building `id`: its left and right sides as shares of its place, its top in radians and its
// height. Most are modest and a few are towers. `gap` scales the space between neighbours.
float4 Building(float id, float base, float low, float high, float gap)
{
    float tall = low + ((high - low) * pow(Hash1(id + 3.1), 2.4));
    float left = gap * (0.04 + (0.14 * Hash1(id + 0.7)));
    float right = 1.0 - (gap * (0.02 + (0.16 * Hash1(id + 1.9))));
    return float4(left, right, base + tall, tall);
}

// Whether building `id` steps in near its top.
bool Stepped(float id)
{
    return Hash1(id + 4.3) < 0.35;
}

// The windows on a distant building's face, over the wall's `colour`. `l` and `r` are its sides
// there, `width` its place's width in radians, `pane` a window with its share of wall.
float3 Windows(float3 colour, float x, float e, float pixel, float clock, float id, float4 b, float base, float l, float r, float width, float2 pane, float bright)
{
    float2 local = float2((x - l) * width, e - base) / pane;
    float2 cell = floor(local);
    float2 f = frac(local);
    float3 wid = float3(cell, id);
    // Offices light whole floors in cool white; homes have scattered windows in warm light.
    float office = step(Hash1(id + 22.2), 0.40);
    float floor_lit = step(Hash2(float2(cell.y, id * 1.7)), lerp(0.06, 0.45, office)) * step(Hash3(wid + 4.4), 0.92);
    float scattered = step(Hash3(wid), lerp(0.34, 0.10, office));
    float lit = Switching(floor(Hash3(wid + 11.0) * 65536.0), clock, max(floor_lit, scattered));
    // The lit pane inside its share of wall, drawn smoothly while it is a few pixels across
    // and as its average once it is smaller, so a far row does not shimmer.
    float frame = Inside(min(min(f.x - 0.20, 0.80 - f.x), min(f.y - 0.26, 0.74 - f.y)) * pane.x, pixel * 0.6);
    frame = lerp(0.29, frame, saturate(((pane.x / pixel) - 1.5) / 2.0));
    // Windows keep off the building's edges and its roof.
    float room = step(0.5, local.x) * step(local.x, ((r - l) * width / pane.x) - 0.5) * step(1.0, local.y) * step(local.y, (b.w / pane.y) - 1.0);
    float tint = Hash3(wid + 3.7);
    float3 warm = lerp(float3(1.00, 0.70, 0.40), float3(1.00, 0.86, 0.62), tint);
    float3 cool = lerp(float3(0.70, 0.86, 1.00), float3(0.94, 0.96, 1.00), tint);
    float3 light = lerp(warm, cool, office) * (0.50 + (0.50 * Hash3(wid + 9.1)));
    return lerp(colour, light * bright, frame * room * lit);
}

// One distant row round the horizon, one building to a place and `cells` places to a full turn.
//   u, e     how far round the horizon (-0.5 to 0.5) and the elevation in radians
//   pixel    a screen pixel in radians
//   base     where the row stands, in radians; it reaches down out of sight below that
//   low, high  the range of heights
//   row      which row, so each has its own buildings
//   body     the colour of the walls
// Returns the row's colour in .rgb and how much it covers in .a.
float4 Skyline(float u, float e, float pixel, float clock, float cells, float base, float low, float high, float gap, float row, float3 body, float2 pane, float bright)
{
    float at = (u + 0.5) * cells;
    float id = fmod(floor(at), cells) + (row * 211.0);
    float x = frac(at);
    float width = TAU / cells;
    float4 b = Building(id, base, low, high, gap);
    float l = b.x;
    float r = b.y;
    if (Stepped(id) && (e > base + (b.w * 0.74)))
    {
        float cut = (r - l) * 0.20;
        l += cut;
        r -= cut;
    }
    // Below where it stands the row is solid: its street and the buildings' feet.
    float cover = max(Inside(min(min(x - l, r - x) * width, b.z - e), pixel), Inside(base - e, pixel));
    float3 colour = body * (1.0 + (0.9 * saturate(1.0 - ((e - base) / max(b.w, 0.001)))));
    colour = Windows(colour, x, e, pixel, clock, id, b, base, l, r, width, pane, bright);
    float4 result = float4(colour, cover);

    // The taller buildings carry a mast with a slowly blinking red light on top.
    float tower = step(lerp(low, high, 0.35), b.w);
    float mast_top = b.z + (0.018 * tower);
    float middle = (l + r) * 0.5;
    float mast = Inside(min(0.0009 - abs((x - middle) * width), min(e - (b.z - 0.002), mast_top - e)), pixel * 0.5) * tower;
    result = lerp(result, float4(body, 1.0), mast);
    float2 tip = float2((x - middle) * width, e - mast_top);
    float lamp = saturate(exp(-dot(tip, tip) / (pixel * pixel * 2.5)) * 1.4) * tower * Beacon(id, clock);
    result.rgb = lerp(result.rgb, float3(1.0, 0.16, 0.12), lamp);
    result.a = max(result.a, lamp);
    return result;
}

// ---------------------------------------------------------------------------------------------
// The words on the signs. The characters are distance fields (neotokyo_glyphs.hlsl): for each a
// grid of how far each point is from its outline, so a sign can draw them at any size.

// One value from a row of a character's distance field, in characters: negative inside. A row
// is 32 values of 4 bits, one uint4.
float RowTexel(uint4 row, uint x)
{
    uint lane = x >> 3;
    uint bits = (lane == 0) ? row.x : ((lane == 1) ? row.y : ((lane == 2) ? row.z : row.w));
    float level = (float)((bits >> ((x & 7) * 4)) & 15);
    return ((level / 15.0) - 0.5) * 2.0 * GLYPH_RANGE;
}

// How far `uv` is from the outline of character `g`, in characters, negative inside. `uv` runs
// 0 to 1 across the character's square from the left and down from the top; beyond the square
// the distance goes on growing. Two rows are read and four values blended; each read of the
// table is costly to build into the shader, so there are only the two, and the whole shader
// calls this once.
float Glyph(uint g, float2 uv)
{
    float2 p = clamp((uv * GLYPH_SIZE) - 0.5, 0.0, GLYPH_SIZE - 1.001);
    uint2 i = (uint2)p;
    float2 f = p - (float2)i;
    uint4 upper = GLYPH_DATA[(g * GLYPH_SIZE) + i.y];
    uint4 lower = GLYPH_DATA[(g * GLYPH_SIZE) + i.y + 1];
    float a = RowTexel(upper, i.x);
    float b = RowTexel(upper, i.x + 1);
    float c = RowTexel(lower, i.x);
    float d = RowTexel(lower, i.x + 1);
    return lerp(lerp(a, b, f.x), lerp(c, d, f.x), f.y) + length(max(abs(uv - 0.5) - 0.5, 0.0));
}

// Which character of a run of text point `p` falls in, and where in it (`uv`). The run is a
// word or a brand's name: where it starts in WORD_TEXT and how long it is. `p` is measured in
// characters from the top-left corner of the first one, across and down. A vertical word runs
// down, and its long vowel mark stands upright as it does in vertical writing.
uint TextChar(float2 p, uint2 w, float vertical, out float2 uv)
{
    float slot = clamp(floor((vertical > 0.5) ? p.y : p.x), 0.0, w.y - 1.0);
    uint g = WORD_TEXT[w.x + (uint)slot];
    uv = (vertical > 0.5) ? float2(p.x, p.y - slot) : float2(p.x - slot, p.y);
    if ((vertical > 0.5) && (g == GLYPH_DASH))
    {
        uv = uv.yx;
    }
    return g;
}

// Which character of the tickers' text point `p` falls in, the text slid along by `slide`
// characters, and where in it (`uv`); TICKER_BLANK in the gaps between words.
uint TickerChar(float2 p, float slide, out float2 uv)
{
    float at = p.x + slide;
    float slot = floor(at);
    float k = slot - (floor(slot / TICKER_LENGTH) * TICKER_LENGTH);
    uv = float2(at - slot, p.y);
    return ((p.y < 0.0) || (p.y > 1.0)) ? TICKER_BLANK : TICKER_TEXT[(uint)k];
}

// A word of no more than `most` characters for thing `id`, or WORD_COUNT when none of those
// tried fits. Each try is a fresh pick, so every word that fits is as likely as any other.
// Words of two characters or more are tried first where they fit, so single kanji stay a
// scattering.
uint PickWord(float id, uint salt, float most)
{
    float least = min(most, 2.0);
    [loop]
    for (uint k = 0; k < 8; k++)
    {
        uint word = min((uint)(Pick(id, salt + k) * WORD_COUNT), WORD_COUNT - 1);
        float n = WORDS[word].y;
        if ((n <= most) && ((n >= least) || (k >= 5)))
        {
            return word;
        }
    }
    return WORD_COUNT;
}

// ---------------------------------------------------------------------------------------------
// The near buildings. Here a point on a facade is `p`: how far across its place from the
// place's left edge, and how high above the street, both in units of the ring's own distance.

// One building of a near ring.
struct Block
{
    float id;
    float present;   // 0 where the place is an open street
    float l;         // its sides, from the place's left edge
    float r;
    float top;       // its roof, above the street
    uint kind;       // 0 offices, 1 flats, 2 a narrow bar building, 3 an old concrete block,
                     // 4 a tower covered in billboards and screens
    float storey;    // one floor's height
    float bays;      // windows across
};

Block NearBlock(float id, float width, float low, float high)
{
    Block b;
    b.id = id;
    b.present = step(0.10, Pick(id, 1u));
    float kind = Pick(id, 5u);
    b.kind = (kind < 0.24) ? 0 : ((kind < 0.42) ? 1 : ((kind < 0.64) ? 2 : ((kind < 0.78) ? 3 : 4)));
    b.l = width * (0.02 + (0.10 * Pick(id, 2u)));
    b.r = width * (1.0 - (0.02 + (0.10 * Pick(id, 3u))));
    // A bar building is narrow, wedged in beside an alley.
    if (b.kind == 2)
    {
        b.r = b.l + ((b.r - b.l) * (0.55 + (0.25 * Pick(id, 4u))));
    }
    b.top = low + ((high - low) * pow(Pick(id, 6u), 1.6));
    b.storey = 0.050 + (0.014 * Pick(id, 7u));
    float bay = ((b.kind == 0) || (b.kind == 4)) ? 0.055 : ((b.kind == 1) ? 0.085 : ((b.kind == 2) ? 1.0 : 0.065));
    b.bays = max(1.0, round((b.r - b.l) / bay));
    return b;
}

// Whether a block of offices steps in near its top.
bool Setback(Block b)
{
    return (b.kind == 0) && (Pick(b.id, 9u) < 0.4);
}

// A sign on a near building.
struct Sign
{
    float4 box;       // left, right, bottom, top
    float size;       // a character's height
    uint word;        // WORD_COUNT until it is picked, when the sign is drawn
    float vertical;   // 1 for a word that runs down
    uint look;        // 0 neon tube, 1 filled neon, 2 ink on a lit box, 3 dots of light,
                      // 4 a stack of small tenants' signs, 5 a video screen, 6 an LED ticker,
                      // 7 an advert poster
    float3 colour;    // the light; on a lit box, the box's
    float3 trim;      // the tube round the edge; on a lit box, the ink
    float id;         // for its flicker, its word and what a screen shows
};

// The margin round the words on a sign, in characters.
static const float PAD = 0.22;

Sign Blank()
{
    Sign s;
    s.box = NO_BOX;
    s.size = 1.0;
    s.word = WORD_COUNT;
    s.vertical = 0.0;
    s.look = 0;
    s.colour = float3(0.0, 0.0, 0.0);
    s.trim = float3(0.0, 0.0, 0.0);
    s.id = 0.0;
    return s;
}

// Whether a sign is there at all.
bool Shown(Sign s)
{
    return s.box.z < s.box.w;
}

// How a sign is lit, and in what colours.
void Dress(inout Sign s, float id)
{
    float pick = Pick(id, 313u);
    s.look = (pick < 0.34) ? 0 : ((pick < 0.66) ? 1 : ((pick < 0.84) ? 2 : 3));
    s.colour = Neon(Pick(id, 324u));
    s.trim = Neon(Pick(id, 335u));
    s.id = id;
    if (s.look == 2)
    {
        // A pale box, lit from inside, with red or near-black lettering.
        s.colour = lerp(float3(0.96, 0.93, 0.92), s.colour, 0.14);
        s.trim = (Pick(id, 346u) < 0.6) ? float3(0.72, 0.05, 0.10) : float3(0.07, 0.05, 0.09);
    }
}

// Which made-up brand an advert is for: 0 Kira Cola, 1 ZAPP, 2 HOSHI, 3 a cup of ramen.
uint Brand(Sign s)
{
    return min((uint)(Pick(s.id, 700u) * 8.0), 7u);
}

// Each brand's poster colour and the colour of its lettering: Kira Cola, ZAPP, HOSHI, KURO
// Coffee, Sora Beer, BIG BUN, ONPU and the ramen.
static const float3 BRAND_GROUND[8] = {
    float3(0.80, 0.04, 0.08), float3(0.02, 0.02, 0.025), float3(0.05, 0.08, 0.32), float3(0.20, 0.10, 0.05),
    float3(0.10, 0.36, 0.72), float3(0.92, 0.40, 0.06), float3(0.28, 0.07, 0.42), float3(0.90, 0.66, 0.08) };
static const float3 BRAND_INK[8] = {
    float3(0.98, 0.97, 0.95), float3(0.55, 1.00, 0.15), float3(0.86, 0.95, 1.00), float3(0.98, 0.80, 0.45),
    float3(0.98, 0.98, 1.00), float3(1.00, 0.96, 0.86), float3(1.00, 0.48, 0.80), float3(0.85, 0.08, 0.06) };

// Makes `s` an advert for its brand: look, colours and lettering.
void Advertise(inout Sign s, float id, float height)
{
    s.look = 7;
    s.id = id;
    s.size = height;
    uint brand = Brand(s);
    s.colour = BRAND_GROUND[brand];
    s.trim = BRAND_INK[brand];
}

// The big sign near the top of tall buildings: one to three characters nearly as wide as the
// building, or a video screen; on a billboard tower, a screen wrapped right across its face.
Sign TowerSign(Block b)
{
    Sign s = Blank();
    if ((b.present < 0.5) || (b.kind == 1) || (b.top < 0.45))
    {
        return s;
    }
    float w = b.r - b.l;
    if (Setback(b))
    {
        w *= 0.64;
    }
    float middle = (b.l + b.r) * 0.5;
    // A tall advert poster for a made-up brand on many, most often on the blank face of an old
    // concrete block.
    if ((b.top > 0.55) && (Pick(b.id, 44u) < ((b.kind == 3) ? 0.72 : 0.42)))
    {
        float across = w * ((b.kind == 2) ? 0.62 : 0.66);
        float height = min(across * 2.0, b.top * 0.55);
        float top = b.top - 0.06;
        s.box = float4(middle - (across * 0.5), middle + (across * 0.5), top - height, top);
        Advertise(s, b.id + 11.0, height);
        return s;
    }
    if ((b.kind == 3) || (b.top < ((b.kind == 4) ? 0.45 : 0.75)) || ((b.kind != 4) && (Pick(b.id, 40u) > 0.6)))
    {
        return s;
    }
    if ((b.kind == 4) || ((b.kind == 0) && (Pick(b.id, 41u) < 0.45)))
    {
        float height = ((b.kind == 4) ? 0.44 : 0.46) * w;
        float span = (b.kind == 4) ? (0.5 * w) + 0.004 : 0.45 * w;
        float top = (b.kind == 4) ? (b.top - 0.03) : (b.top * (0.62 - (0.10 * Pick(b.id, 43u))));
        s.box = float4(middle - span, middle + span, top - height, top);
        Dress(s, b.id + 3.0);
        s.look = 5;
        s.size = height;
        return s;
    }
    float size = 0.40 * w;
    uint word = PickWord(b.id, 440u, min(floor(((b.top * 0.40) / size) - (2.0 * PAD)), 3.0));
    if (word >= WORD_COUNT)
    {
        return s;
    }
    float n = WORDS[word].y;
    float across = (1.0 + (2.0 * PAD)) * size;
    float top = b.top - 0.05;
    s.box = float4(middle - (across * 0.5), middle + (across * 0.5), top - ((n + (2.0 * PAD)) * size), top);
    s.size = size;
    s.word = word;
    s.vertical = 1.0;
    Dress(s, b.id + 3.0);
    return s;
}

// The sign that hangs out from one side of many buildings, upright, reaching past the wall. On
// a bar building it is a tall stack of the tenants' small signs.
Sign SideSign(Block b, float width, Sign tower)
{
    Sign s = Blank();
    if ((b.present < 0.5) || (b.top < 0.30) || (Pick(b.id, 50u) > ((b.kind == 2) ? 1.0 : 0.70)))
    {
        return s;
    }
    float w = b.r - b.l;
    float size = clamp(0.15 * w, 0.030, 0.056);
    float across = (1.0 + (2.0 * PAD)) * size;
    float left = (Pick(b.id, 51u) < 0.5) ? (b.l - (0.30 * across)) : (b.r - (0.70 * across));
    left = clamp(left, 0.005, width - across - 0.005);
    float bottom = GROUND + 0.035;
    float top = min(b.top - 0.04, bottom + (b.top * (0.45 + (0.35 * Pick(b.id, 52u)))));
    top = Shown(tower) ? min(top, tower.box.z - 0.012) : top;
    if (top - bottom < 2.0 * size)
    {
        return s;
    }
    s.size = size;
    s.vertical = 1.0;
    s.id = b.id + 5.0;
    if (b.kind == 2)
    {
        s.look = 4;
        s.box = float4(left, left + across, bottom, top);
        return s;
    }
    uint word = PickWord(b.id, 530u, min(floor(((top - bottom) / size) - (2.0 * PAD)), 5.0));
    if (word >= WORD_COUNT)
    {
        return s;
    }
    float n = WORDS[word].y;
    s.box = float4(left, left + across, top - ((n + (2.0 * PAD)) * size), top);
    s.word = word;
    Dress(s, b.id + 5.0);
    return s;
}

// The board that stands on the roof of some lower buildings, the word running across it. Not
// on a building that has a tower sign.
Sign RoofSign(Block b, float width, Sign tower)
{
    Sign s = Blank();
    if ((b.present < 0.5) || (b.top > 1.1) || (Pick(b.id, 60u) > 0.5) || Shown(tower))
    {
        return s;
    }
    float w = b.r - b.l;
    // Some roofs carry an advert billboard rather than a word.
    if (Pick(b.id, 63u) < 0.45)
    {
        float wide = min(0.94 * w, width - 0.01);
        float tall = clamp(0.40 * wide, 0.07, 0.16);
        float centre = clamp((b.l + b.r) * 0.5, (wide * 0.5) + 0.005, width - (wide * 0.5) - 0.005);
        float foot = b.top + 0.03;
        s.box = float4(centre - (wide * 0.5), centre + (wide * 0.5), foot, foot + tall);
        Advertise(s, b.id + 13.0, tall);
        return s;
    }
    float size = 0.045 + (0.035 * Pick(b.id, 61u));
    uint word = PickWord(b.id, 620u, floor(((0.98 * w) / size) - (2.0 * PAD)));
    if (word >= WORD_COUNT)
    {
        return s;
    }
    float n = WORDS[word].y;
    size = min(size, (0.98 * w) / (n + (2.0 * PAD)));
    float across = (n + (2.0 * PAD)) * size;
    float middle = clamp((b.l + b.r) * 0.5, (across * 0.5) + 0.005, width - (across * 0.5) - 0.005);
    float bottom = b.top + 0.03;
    s.box = float4(middle - (across * 0.5), middle + (across * 0.5), bottom, bottom + ((1.0 + (2.0 * PAD)) * size));
    s.size = size;
    s.word = word;
    Dress(s, b.id + 6.0);
    return s;
}

// How tall a band of floors is on a building whose face is covered in panels.
float BandHeight(Block b)
{
    return b.storey * (2.0 + floor(Pick(b.id, 90u) * 2.0));
}

// The panel on the band of floors at height `h`: a billboard with a word, a small video screen,
// or an LED ticker along the ledge, on most bands of a billboard tower and on some bands of the
// others. Panels stay below `ceiling`, where the tower sign starts, and clear of the side sign.
// The billboard's word is picked when it is drawn.
Sign BandSign(Block b, float h, float ceiling, Sign side)
{
    Sign s = Blank();
    float chance = (b.kind == 4) ? 0.95 : ((b.kind == 2) ? 0.40 : ((b.kind == 0) ? 0.32 : ((b.kind == 3) ? 0.28 : 0.10)));
    float band = BandHeight(b);
    float k = floor((h - GROUND - 0.02) / band);
    float bottom = GROUND + 0.02 + (k * band);
    float roof = Setback(b) ? (b.top * 0.80) : (b.top - 0.05);
    float band_id = (b.id * 64.0) + k;
    if ((b.present < 0.5) || (k < 0.0) || (bottom + band > min(roof, ceiling)) || (Pick(band_id, 91u) > chance))
    {
        return s;
    }
    float l = b.l + 0.006;
    float r = b.r - 0.006;
    if (Shown(side) && (side.box.w > bottom) && (side.box.z < bottom + band))
    {
        if (side.box.x < (b.l + b.r) * 0.5)
        {
            l = max(l, side.box.y + 0.008);
        }
        else
        {
            r = min(r, side.box.x - 0.008);
        }
    }
    if (r - l < 0.06)
    {
        return s;
    }
    float pick = Pick(band_id, 92u);
    if (pick < 0.22)
    {
        // An LED ticker along the ledge, its text sliding to and fro with the clock as some
        // LED signs bounce a message.
        float height = band * 0.26;
        float base = bottom + (band * 0.04);
        s.box = float4(l, r, base, base + height);
        s.size = height * 0.80;
        s.look = 6;
        s.id = band_id;
        float hue = Pick(band_id, 93u);
        s.colour = (hue < 0.5) ? float3(1.00, 0.62, 0.16) : ((hue < 0.8) ? float3(1.00, 0.20, 0.14) : float3(0.30, 1.00, 0.45));
        s.trim = s.colour;
        return s;
    }
    float share = (b.kind == 4) ? 1.0 : (0.55 + (0.45 * Pick(band_id, 94u)));
    float across = (r - l) * share;
    float left = l + (((r - l) - across) * Pick(band_id, 95u));
    float height = band * (0.62 + (0.22 * Pick(band_id, 96u)));
    float base = bottom + ((band - height) * 0.5);
    s.box = float4(left, left + across, base, base + height);
    Dress(s, band_id);
    if (pick < 0.38)
    {
        s.look = 5;
        s.size = height;
    }
    else if (pick < 0.62)
    {
        Advertise(s, band_id + 0.5, height);
    }
    else
    {
        s.size = height / (1.0 + (2.0 * PAD));
    }
    return s;
}

// How many shops share the ground floor.
float Shops(Block b)
{
    return 1.0 + floor(Pick(b.id, 20u) * 3.0);
}

// The sign over the shop the point is in front of.
Sign ShopSign(Block b, float x)
{
    Sign s = Blank();
    float shops = Shops(b);
    float span = (b.r - b.l) / shops;
    float k = clamp(floor((x - b.l) / span), 0.0, shops - 1.0);
    float shop = (b.id * 4.0) + k;
    if ((b.present < 0.5) || (Pick(shop, 70u) > 0.8))
    {
        return s;
    }
    float size = 0.022 / (1.0 + (2.0 * PAD));
    uint word = PickWord(shop, 700u, floor(((0.9 * span) / size) - (2.0 * PAD)));
    if (word >= WORD_COUNT)
    {
        return s;
    }
    float across = (WORDS[word].y + (2.0 * PAD)) * size;
    float middle = b.l + ((k + 0.5) * span);
    s.box = float4(middle - (across * 0.5), middle + (across * 0.5), GROUND - 0.026, GROUND - 0.004);
    s.size = size;
    s.word = word;
    Dress(s, shop + 9.0);
    return s;
}

// ---------------------------------------------------------------------------------------------
// What the video screens show: an idol's smiling face, a waving rice-ball mascot, a cold drink,
// or slow waves of colour, each swaying, blinking or waving with the eased clock. `c` is the
// point against the picture, -1 to 1 across and up, and `px` a screen pixel in those units.

// How much of a shape covers a point, from its distance `d` (negative inside), a pixel `px` wide.
float Fill(float d, float px)
{
    return saturate(0.5 - (d / px));
}

// An idol's face, smiling and blinking now and then, her head tilting a little.
float3 Idol(float2 c, float px, float clock, float seed, float3 background)
{
    float sway = Ease(clock) * TAU;
    c.y -= 0.04 * sin((sway * 2.0) + (seed * 3.0));
    float tilt = 0.10 * sin((sway * 1.3) + (seed * 6.0));
    c = float2((c.x * cos(tilt)) - (c.y * sin(tilt)), (c.x * sin(tilt)) + (c.y * cos(tilt)));
    float hue = frac(seed * 7.3);
    float3 hair = (hue < 0.3) ? float3(0.18, 0.09, 0.07) : ((hue < 0.55) ? float3(0.95, 0.48, 0.68) : ((hue < 0.8) ? float3(0.07, 0.07, 0.15) : float3(0.95, 0.80, 0.48)));
    float3 skin = float3(1.00, 0.86, 0.77);
    float3 colour = background;
    // Hair behind the face, down to the shoulders, and her shoulders in a bright top.
    float crown = length((c - float2(0.0, 0.12)) / float2(0.92, 0.92)) - 1.0;
    float2 fall = abs(c - float2(0.0, -0.38)) - float2(0.86, 0.52);
    float behind = min(crown, length(max(fall, 0.0)) + min(max(fall.x, fall.y), 0.0) - 0.06);
    colour = lerp(colour, hair * 0.85, Fill(behind, px));
    float shoulders = length((c - float2(0.0, -1.35)) / float2(1.05, 0.55)) - 1.0;
    colour = lerp(colour, lerp(float3(1.0, 1.0, 1.0), Neon(frac(seed * 3.1)), 0.5) * 0.9, Fill(shoulders * 0.5, px));
    // The face.
    float face = (length((c - float2(0.0, -0.08)) / float2(0.68, 0.80)) - 1.0) * 0.7;
    colour = lerp(colour, skin, Fill(face, px));
    // A fringe over the forehead, cut in soft points.
    float fringe = (0.32 - (0.07 * abs(sin(c.x * 7.5)))) - c.y;
    colour = lerp(colour, hair, Fill(max(fringe, crown), px));
    // Eyes, both from one: big and dark with a bright catch-light, closing for a blink.
    float beat = frac((Ease(clock) * 3.0) + seed);
    float open = 1.0 - (0.88 * smoothstep(0.80, 0.84, beat) * (1.0 - smoothstep(0.88, 0.92, beat)));
    float2 e = float2(abs(c.x), c.y) - float2(0.29, 0.02);
    float eye = (length(e / float2(0.13, (0.17 * open) + 0.015)) - 1.0) * 0.13;
    colour = lerp(colour, float3(0.10, 0.06, 0.16), Fill(eye, px));
    float iris = (length((e + float2(0.0, 0.03)) / float2(0.095, (0.12 * open) + 0.01)) - 1.0) * 0.095;
    colour = lerp(colour, lerp(float3(0.55, 0.30, 0.85), float3(0.20, 0.10, 0.40), saturate((e.y + 0.1) * 4.0)), Fill(iris, px) * step(0.3, open));
    float catchlight = length(e - float2(0.035, 0.06)) - 0.035;
    colour = lerp(colour, float3(1.0, 1.0, 1.0), Fill(catchlight, px) * step(0.3, open));
    // Pink cheeks and an open smile.
    colour += float3(0.30, 0.04, 0.08) * exp(-dot(e - float2(0.12, -0.26), e - float2(0.12, -0.26)) / 0.012);
    float2 m = (c - float2(0.0, -0.40)) / float2(0.15, 0.11);
    float smile = max(length(m) - 1.0, m.y) * 0.11;
    colour = lerp(colour, float3(0.62, 0.12, 0.18), Fill(smile, px));
    float tongue = (length((c - float2(0.0, -0.48)) / float2(0.08, 0.04)) - 1.0) * 0.04;
    colour = lerp(colour, float3(1.0, 0.45, 0.50), Fill(max(tongue, smile), px));
    return colour;
}

// The distance to a rounded triangle, pointing up, `r` across: the shape of a rice ball.
float RiceBall(float2 p, float r)
{
    float k = sqrt(3.0);
    p.x = abs(p.x) - r;
    p.y = p.y + (r / k);
    if ((p.x + (k * p.y)) > 0.0)
    {
        p = float2(p.x - (k * p.y), (-k * p.x) - p.y) / 2.0;
    }
    p.x -= clamp(p.x, -2.0 * r, 0.0);
    return -length(p) * sign(p.y);
}

// A rice-ball mascot, hopping and waving.
float3 Mascot(float2 c, float px, float clock, float seed, float3 background)
{
    float sway = Ease(clock) * TAU;
    c.y -= 0.06 * abs(sin((sway * 2.5) + (seed * 5.0)));
    float3 colour = background;
    // A little arm, waving.
    float wave = 0.6 + (0.5 * sin((sway * 3.0) + seed));
    float2 a = c - float2(0.55, -0.20);
    a = float2((a.x * cos(wave)) + (a.y * sin(wave)), (-a.x * sin(wave)) + (a.y * cos(wave)));
    float arm = length(a - float2(0.0, clamp(a.y, 0.0, 0.30))) - 0.08;
    colour = lerp(colour, float3(0.97, 0.96, 0.92), Fill(arm, px));
    // The body, and its strip of seaweed.
    float body = RiceBall(c - float2(0.0, -0.10), 0.62) - 0.20;
    colour = lerp(colour, float3(0.97, 0.96, 0.92), Fill(body, px));
    float2 band = abs(c - float2(0.0, -0.62)) - float2(0.30, 0.20);
    float nori = max(length(max(band, 0.0)) + min(max(band.x, band.y), 0.0), body);
    colour = lerp(colour, float3(0.06, 0.12, 0.08), Fill(nori, px));
    // Dot eyes, a small smile and pink cheeks.
    float2 e = float2(abs(c.x), c.y) - float2(0.19, -0.02);
    float eye = length(e / float2(0.055, 0.075)) - 1.0;
    colour = lerp(colour, float3(0.05, 0.05, 0.06), Fill(eye * 0.055, px));
    colour = lerp(colour, float3(1.0, 1.0, 1.0), Fill(length(e - float2(0.015, 0.025)) - 0.018, px));
    float mouth = abs(length(c - float2(0.0, -0.10)) - 0.09) - 0.018;
    colour = lerp(colour, float3(0.45, 0.10, 0.12), Fill(max(mouth, c.y + 0.10), px));
    colour += float3(0.35, 0.05, 0.10) * exp(-dot(e - float2(0.12, -0.16), e - float2(0.12, -0.16)) / 0.006) * Fill(body, px);
    return colour;
}

// A cold can of drink with a shine running down it and a few bubbles round it.
float3 Drink(float2 c, float px, float clock, float seed, float3 background, float3 brand)
{
    float sway = Ease(clock) * TAU;
    float3 colour = background;
    float2 can = abs(c - float2(0.0, -0.02)) - float2(0.30, 0.68);
    float shape = length(max(can, 0.0)) + min(max(can.x, can.y), 0.0) - 0.08;
    float3 metal = lerp(float3(0.55, 0.58, 0.62), float3(0.95, 0.96, 0.98), 0.5 + (0.5 * cos(c.x * 6.0)));
    metal = lerp(metal, brand, step(abs(c.y + 0.05), 0.30) * 0.85);
    metal += 0.5 * exp(-pow((c.x + 0.14 + (0.04 * sin(sway))) / 0.05, 2.0));
    colour = lerp(colour, metal, Fill(shape, px));
    // Bubbles round it.
    float2 cell = floor((c + 2.0) * 3.0);
    float2 local = frac((c + 2.0) * 3.0) - 0.5;
    float2 jitter = (float2(Pick3(cell, seed * 97.0, 61u), Pick3(cell, seed * 97.0, 62u)) - 0.5) * 0.5;
    float bubble = abs(length(local - jitter) - 0.12) - 0.02;
    colour = lerp(colour, float3(0.9, 0.97, 1.0), Fill(bubble / 3.0, px) * step(0.55, Pick3(cell, seed * 97.0, 63u)) * step(0.0, shape));
    return colour;
}

// What a screen shows at point `c`, -1 to 1 across and up a square picture.
float3 Picture(Sign s, float2 c, float px, float clock)
{
    float sway = Ease(clock) * TAU;
    float seed = Pick(s.id, 500u);
    float choice = Pick(s.id, 501u);
    // Behind everything, a soft gradient in the screen's colours with a slow sheen.
    float3 background = lerp(s.colour, s.trim, saturate(0.5 + (0.35 * (c.x + c.y)) + (0.15 * sin(sway + (c.y * 2.0)))));
    background = lerp(background, float3(1.0, 1.0, 1.0), 0.10) * 0.55;
    float3 colour;
    [branch]
    if (choice < 0.38)
    {
        colour = Idol(c, px, clock, seed, background);
    }
    else if (choice < 0.68)
    {
        colour = Mascot(c, px, clock, seed, background);
    }
    else if (choice < 0.86)
    {
        colour = Drink(c, px, clock, seed, background, Neon(frac(seed * 5.7)));
    }
    else
    {
        float wave = sin((c.x * 2.6) + (sway * 1.2) + (1.5 * sin((c.y * 3.0) + (sway * 0.7))));
        colour = lerp(s.colour, s.trim, 0.5 + (0.5 * wave)) * 0.65;
    }
    return colour;
}

// ---------------------------------------------------------------------------------------------
// The adverts' products. `c` is the point against the product, -1 to 1 across, and `px` a
// screen pixel in those units.

// The distance to a box `half_` across and up round the origin, its corners rounded by `r`.
float RoundBox(float2 p, float2 half_, float r)
{
    float2 q = abs(p) - half_ + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

// Two shapes joined smoothly over `k`.
float SmoothMin(float a, float b, float k)
{
    float h = saturate(0.5 + (0.5 * (b - a) / k));
    return lerp(b, a, h) - (k * h * (1.0 - h));
}

// The distance to the line from `a` to `b`.
float Segment(float2 p, float2 a, float2 b)
{
    float2 pa = p - a;
    float2 ba = b - a;
    return length(pa - (ba * saturate(dot(pa, ba) / dot(ba, ba))));
}

// A sparkle of light here and there round a product, each brightening and fading at its own
// pace with the clock.
float Sparkles(float2 c, float px, float clock, float density, uint salt)
{
    float2 cell = floor(c * density);
    float2 f = frac(c * density) - 0.5;
    float2 spot = (float2(Pick3(cell, 7.0, salt), Pick3(cell, 7.0, salt + 1u)) - 0.5) * 0.5;
    float pulse = 0.5 + (0.5 * cos((Ease(clock) * TAU * 2.0) + (Pick3(cell, 7.0, salt + 2u) * TAU)));
    float dot_ = length(f - spot) - 0.10;
    return Fill(dot_ / density, px) * step(0.6, Pick3(cell, 7.0, salt + 3u)) * pulse;
}

// Where a point `x` across the face of a cylinder `r` across lies round it, as an angle from
// the front, once the cylinder has turned by `turn`: -PI to PI.
float Around(float x, float r, float turn)
{
    float a = asin(clamp(x / r, -1.0, 1.0)) + turn;
    return atan2(sin(a), cos(a));
}

// The distance to a four-pointed sparkle `r` across.
float SparkleShape(float2 p, float r)
{
    return length(p) - (r * (0.22 + (0.78 * pow(abs(cos(atan2(p.y, p.x) * 2.0)), 6.0))));
}

// What an advert poster shows behind its lettering: the brand's colour, lighter toward the
// middle with rays behind the product, and its product in the middle.
//
// The product spins in as the clock swings up past this poster's own point in the swing (from
// `seed`), growing from nothing through two quick turns to a stop with a little overshoot and
// a burst of light, and spins out again as the clock comes back down; so where the clock turns
// round at zero it is not there, and the posters do not all spin at once. Settled, it turns
// slowly to and fro like a display stand, showing its label: the label and its marks move
// round the bottle or can while the light on it stays where it is.
float3 Advert(uint brand, float2 c, float px, float clock, float3 ground, float seed)
{
    float sway = Ease(clock) * TAU;
    float3 colour = ground * (0.72 + (0.40 * exp(-dot(c, c) * 0.6)));
    float rays = 0.5 + (0.5 * sin((atan2(c.y - 0.1, c.x) * 16.0) + (sway * 0.3)));
    colour *= 0.90 + (0.12 * rays);
    float grow = saturate((clock - (0.04 + (0.26 * seed))) / 0.14);
    float settle = grow * grow * (3.0 - (2.0 * grow));
    [branch]
    if (settle <= 0.0)
    {
        return colour;
    }
    float scale = max(settle + (0.12 * sin(settle * PI)), 0.02);
    float turn = ((1.0 - settle) * (1.0 - settle) * TAU * 2.0) + (0.8 * sin((sway * 0.8) + (seed * TAU)));
    float2 q = c / scale;
    float qx = px / scale;
    [branch]
    if (brand == 0)
    {
        // Kira Cola: a cold glass bottle with a gold label marked front and back with a white
        // sparkle (kira), beaded with condensation, and bubbles sparkling round it. Its own
        // look, so it is not taken for any real cola.
        float neck = RoundBox(q - float2(0.0, 0.42), float2(0.085, 0.25), 0.04);
        float body = RoundBox(q - float2(0.0, -0.32), float2(0.27, 0.55), 0.13);
        float bottle = SmoothMin(body, neck, 0.18);
        float cap = RoundBox(q - float2(0.0, 0.70), float2(0.10, 0.04), 0.015);
        float r = lerp(0.27, 0.085, smoothstep(-0.02, 0.30, q.y));
        float u = Around(q.x, r, turn);
        float facing = sqrt(saturate(1.0 - ((q.x / r) * (q.x / r))));
        float3 glass = lerp(float3(0.22, 0.07, 0.04), float3(0.50, 0.18, 0.08), saturate(0.5 + (q.x * 2.5)));
        glass += float3(0.9, 0.85, 0.8) * 0.75 * exp(-pow((q.x + 0.12) / 0.035, 2.0));
        glass += float3(0.9, 0.85, 0.8) * 0.25 * exp(-pow((q.x - 0.16) / 0.025, 2.0));
        float label = step(abs(q.y + 0.30), 0.16);
        glass = lerp(glass, lerp(float3(0.70, 0.48, 0.10), float3(1.0, 0.86, 0.45), facing * saturate(0.6 - q.x)), label);
        float sparkle = min(SparkleShape(float2(u * r, q.y + 0.30), 0.11), SparkleShape(float2(Around(q.x, r, turn + PI) * r, q.y + 0.30), 0.11));
        glass = lerp(glass, float3(1.0, 1.0, 0.98), Fill(sparkle * max(facing, 0.2), qx) * label);
        glass += 0.30 * Sparkles(float2(u * r, q.y), qx, 0.0, 14.0, 81u) * (1.0 - label) * facing;
        colour = lerp(colour, glass, Fill(bottle, qx));
        colour = lerp(colour, float3(0.75, 0.72, 0.70), Fill(cap, qx));
        colour += float3(1.0, 0.95, 0.9) * 0.6 * Sparkles(c * 0.9, px, clock, 5.0, 91u) * step(0.0, bottle);
    }
    else if (brand == 1)
    {
        // ZAPP: a slim black can with a green lightning bolt front and back, and green sparks
        // round it.
        float can = RoundBox(q - float2(0.0, -0.05), float2(0.20, 0.72), 0.06);
        float u = Around(q.x, 0.20, turn);
        float facing = sqrt(saturate(1.0 - ((q.x / 0.20) * (q.x / 0.20))));
        float3 metal = lerp(float3(0.04, 0.04, 0.05), float3(0.34, 0.36, 0.38), pow(saturate(1.0 - (abs(q.x + 0.07) / 0.20)), 3.0));
        float2 front = float2(u * 0.20, q.y);
        float2 back = float2(Around(q.x, 0.20, turn + PI) * 0.20, q.y);
        float bolt = 1.0;
        [unroll]
        for (int k = 0; k < 2; k++)
        {
            float2 m = (k == 0) ? front : back;
            bolt = min(bolt, min(min(Segment(m, float2(0.07, 0.42), float2(-0.08, 0.02)), Segment(m, float2(-0.08, 0.02), float2(0.07, 0.02))), Segment(m, float2(0.07, 0.02), float2(-0.07, -0.42))) - 0.045);
        }
        metal = lerp(metal, float3(0.55, 1.00, 0.15) * (0.6 + (0.4 * facing)), Fill(bolt * max(facing, 0.2), qx));
        metal = lerp(metal, float3(0.55, 0.57, 0.60), step(0.64, abs(q.y + 0.05)));
        colour = lerp(colour, metal, Fill(can, qx));
        colour += float3(0.55, 1.00, 0.15) * 0.8 * Sparkles(c, px, clock, 6.0, 101u) * step(0.0, can);
    }
    else if (brand == 2)
    {
        // HOSHI: a phone, flat, so turning it narrows it and spinning shows its back. On the
        // front its screen glows with a star and a sheen sweeping across; on the back a camera
        // and a small star.
        float face = cos(turn);
        float side = abs(face);
        float width = max(0.34 * side, 0.022);
        float phone = RoundBox(q - float2(0.0, -0.05), float2(width, 0.68), min(0.09, width));
        float2 f = float2(q.x / max(side, 0.065), q.y);
        float3 body = float3(0.03, 0.03, 0.04);
        [branch]
        if (face > 0.0)
        {
            float glass = RoundBox(f - float2(0.0, -0.05), float2(0.30, 0.63), 0.06);
            float3 screen = lerp(float3(0.30, 0.10, 0.62), float3(0.10, 0.72, 0.95), saturate(0.5 + (f.y * 0.8)));
            float2 st = f - float2(0.0, 0.02);
            float star = length(st) - (0.17 * (0.55 + (0.45 * pow(abs(cos(atan2(st.x, st.y) * 2.5)), 2.0))));
            screen = lerp(screen, float3(1.0, 0.97, 0.85), Fill(star * side, qx));
            screen += 0.25 * exp(-pow(((f.x + (f.y * 0.5)) - (0.8 * sin(sway * 0.8))) / 0.10, 2.0));
            screen *= 0.55 + (0.45 * side);
            body = lerp(body, screen, Fill(glass * side, qx));
            body = lerp(body, float3(0.02, 0.02, 0.03), Fill((length(f - float2(0.0, 0.53)) - 0.02) * side, qx));
        }
        else
        {
            body = float3(0.10, 0.11, 0.13) * (0.6 + (0.4 * side));
            body = lerp(body, float3(0.02, 0.02, 0.025), Fill((RoundBox(f - float2(0.17, 0.45), float2(0.09, 0.12), 0.05)) * side, qx));
            body = lerp(body, float3(0.75, 0.80, 0.90), Fill((length(f - float2(0.0, -0.05)) - (0.07 * (0.55 + (0.45 * pow(abs(cos(atan2(f.x, f.y + 0.05) * 2.5)), 2.0))))) * side, qx));
        }
        colour = lerp(colour, body, Fill(phone, qx));
    }
    else if (brand == 3)
    {
        // KURO Coffee: a short can of black coffee, a gold band round it with a coffee bean
        // printed front and back.
        float can = RoundBox(q - float2(0.0, -0.10), float2(0.26, 0.52), 0.05);
        float facing = sqrt(saturate(1.0 - ((q.x / 0.26) * (q.x / 0.26))));
        float3 metal = lerp(float3(0.10, 0.06, 0.04), float3(0.40, 0.25, 0.15), pow(saturate(1.0 - (abs(q.x + 0.08) / 0.26)), 2.0));
        float band = step(abs(q.y + 0.08), 0.18);
        metal = lerp(metal, float3(0.80, 0.60, 0.25) * (0.55 + (0.45 * facing)), band);
        float bean = 1.0;
        float seam = 1.0;
        [unroll]
        for (int k = 0; k < 2; k++)
        {
            float2 m = float2(Around(q.x, 0.26, turn + (k * PI)) * 0.26, q.y + 0.08);
            float shape = (length(m / float2(0.07, 0.10)) - 1.0) * 0.07;
            bean = min(bean, shape);
            seam = min(seam, max(abs(m.x - (0.02 * sin(m.y * 30.0))) - 0.007, shape));
        }
        metal = lerp(metal, float3(0.22, 0.11, 0.05), Fill(bean * max(facing, 0.2), qx) * band);
        metal = lerp(metal, float3(0.80, 0.60, 0.25), Fill(seam * max(facing, 0.2), qx) * band);
        metal = lerp(metal, float3(0.62, 0.60, 0.58), step(0.47, abs(q.y + 0.10)));
        colour = lerp(colour, metal, Fill(can, qx));
    }
    else if (brand == 4)
    {
        // Sora Beer: a tall glass of golden beer under a thick head of foam, bubbles rising in
        // it, a small star printed on the glass going round as it turns.
        float flare = lerp(0.20, 0.30, saturate((q.y + 0.75) / 1.30));
        float glass = max(abs(q.x) - flare, max(q.y - 0.55, -0.75 - q.y));
        float facing = sqrt(saturate(1.0 - ((q.x / flare) * (q.x / flare))));
        float3 beer = lerp(float3(0.80, 0.42, 0.04), float3(1.00, 0.76, 0.20), saturate(0.5 + (q.y * 0.5))) * (0.70 + (0.30 * facing));
        beer += float3(1.0, 0.95, 0.80) * 0.5 * Sparkles(float2(Around(q.x, flare, turn) * flare, q.y), qx, clock, 9.0, 141u);
        beer += float3(1.0, 1.0, 0.95) * 0.45 * exp(-pow((q.x + 0.10) / 0.03, 2.0));
        float star = min(SparkleShape(float2(Around(q.x, flare, turn) * flare, q.y + 0.05), 0.09), SparkleShape(float2(Around(q.x, flare, turn + PI) * flare, q.y + 0.05), 0.09));
        beer = lerp(beer, float3(1.0, 1.0, 1.0), Fill(star * max(facing, 0.2), qx));
        colour = lerp(colour, beer, Fill(glass, qx));
        float foam = max(abs(q.x) - (flare + 0.02), max(q.y - (0.66 + (0.035 * sin(q.x * 26.0))), 0.42 - q.y));
        colour = lerp(colour, float3(0.98, 0.96, 0.90) * (0.80 + (0.20 * facing)), Fill(foam, qx));
    }
    else if (brand == 5)
    {
        // BIG BUN: a burger stacked high, the sesame seeds on its bun going round as it turns.
        float bottom = RoundBox(q - float2(0.0, -0.56), float2(0.48, 0.10), 0.08);
        float patty = RoundBox(q - float2(0.0, -0.37), float2(0.51, 0.08), 0.07);
        float cheese = min(RoundBox(q - float2(0.0, -0.27), float2(0.53, 0.025), 0.01), max(abs(q.y + 0.31) - 0.05, abs(frac((q.x * 3.0) + 0.5) - 0.5) - 0.08) );
        cheese = max(cheese, abs(q.x) - 0.53);
        float lettuce = max(abs(q.y + 0.20 + (0.025 * sin(q.x * 24.0))) - 0.035, abs(q.x) - 0.55);
        float tomato = RoundBox(q - float2(0.0, -0.12), float2(0.46, 0.035), 0.03);
        float dome = max((length((q - float2(0.0, -0.08)) / float2(0.52, 0.48)) - 1.0) * 0.48, -(q.y + 0.08));
        colour = lerp(colour, float3(0.82, 0.50, 0.20), Fill(bottom, qx));
        colour = lerp(colour, float3(0.30, 0.14, 0.07), Fill(patty, qx));
        colour = lerp(colour, float3(1.00, 0.78, 0.14), Fill(cheese, qx));
        colour = lerp(colour, float3(0.36, 0.74, 0.20), Fill(lettuce, qx));
        colour = lerp(colour, float3(0.86, 0.16, 0.10), Fill(tomato, qx));
        float r = 0.52 * sqrt(saturate(1.0 - pow((q.y + 0.08) / 0.48, 2.0)));
        float3 bun = lerp(float3(0.78, 0.45, 0.16), float3(0.98, 0.70, 0.35), saturate(0.4 + (q.y * 1.2) - (q.x * 0.6)));
        float2 seat = float2(Around(q.x, max(r, 0.01), turn) * 0.5, q.y) * 12.0;
        float2 cell = floor(seat);
        float seed = (length((frac(seat) - 0.5 - ((float2(Pick3(cell, 5.0, 151u), Pick3(cell, 5.0, 152u)) - 0.5) * 0.4)) / float2(0.14, 0.22)) - 1.0) * 0.14 / 12.0;
        bun = lerp(bun, float3(1.0, 0.95, 0.82), Fill(seed, qx) * step(Pick3(cell, 5.0, 153u), 0.45) * step(0.0, q.y));
        colour = lerp(colour, bun, Fill(dome, qx));
    }
    else if (brand == 6)
    {
        // ONPU: a pair of headphones turning on their band, with notes floating round them.
        float side = cos(turn);
        float squash = max(abs(side), 0.15);
        float2 h = float2(q.x / squash, q.y);
        float band = max(abs(length(h - float2(0.0, -0.05)) - 0.44) - 0.045, -(h.y + 0.05));
        float cup = RoundBox(float2(abs(h.x), h.y) - float2(0.44, -0.26), float2(0.11, 0.19), 0.09);
        float pad = RoundBox(float2(abs(h.x), h.y) - float2(0.40, -0.26), float2(0.05, 0.15), 0.05);
        colour = lerp(colour, float3(0.10, 0.10, 0.12), Fill(band * squash, qx));
        colour = lerp(colour, lerp(float3(0.95, 0.40, 0.72), float3(1.0, 0.75, 0.90), saturate(0.5 - (q.x * 2.0))), Fill(cup * squash, qx));
        colour = lerp(colour, float3(0.12, 0.10, 0.14), Fill(pad * squash, qx) * step(0.0, side * h.x));
        float2 cell = floor(c * 3.0);
        float2 f = frac(c * 3.0) - 0.5;
        f.y -= 0.08 * sin((sway * 1.5) + (Pick3(cell, 9.0, 132u) * TAU));
        float note = min((length((f - float2(-0.06, -0.12)) / float2(0.10, 0.075)) - 1.0) * 0.075, RoundBox(f - float2(0.03, 0.04), float2(0.012, 0.16), 0.005));
        colour = lerp(colour, float3(1.0, 0.85, 0.95), Fill(note / 3.0, px) * step(0.55, Pick3(cell, 9.0, 131u)) * step(0.0, min(band, cup)));
    }
    else
    {
        // A cup of ramen, steaming, its lid peeled back, a round emblem going round on its band.
        float flare = lerp(0.30, 0.42, saturate((q.y + 0.70) / 0.95));
        float cup = max(abs(q.x) - flare, max(q.y - 0.25, -0.70 - q.y));
        float u = Around(q.x, flare, turn);
        float facing = sqrt(saturate(1.0 - ((q.x / flare) * (q.x / flare))));
        float3 paper = float3(0.97, 0.96, 0.93);
        float band = step(abs(q.y + 0.20), 0.15);
        paper = lerp(paper, float3(0.85, 0.08, 0.06), band);
        paper = lerp(paper, float3(1.00, 0.80, 0.10), step(abs(q.y + 0.20), 0.03) * band);
        float emblem = min(length(float2(u * flare, q.y + 0.20)), length(float2(Around(q.x, flare, turn + PI) * flare, q.y + 0.20))) - 0.10;
        paper = lerp(paper, float3(1.00, 0.80, 0.10), Fill(emblem * max(facing, 0.2), qx));
        paper = lerp(paper, float3(0.85, 0.08, 0.06), Fill((abs(emblem + 0.04) - 0.012) * max(facing, 0.2), qx));
        paper *= 0.75 + (0.25 * facing);
        colour = lerp(colour, paper, Fill(cup, qx));
        float lid = (length((q - float2(0.10, 0.36)) / float2(0.40, 0.10)) - 1.0) * 0.10;
        colour = lerp(colour, float3(0.92, 0.88, 0.80), Fill(lid, qx));
        float steam = 1.0;
        [unroll]
        for (int k = -1; k <= 1; k++)
        {
            float x = (k * 0.16) + (0.05 * sin((q.y * 8.0) + (sway * 1.5) + k));
            steam = min(steam, abs(q.x - x) - 0.022);
        }
        colour = lerp(colour, float3(1.0, 1.0, 1.0), Fill(steam, qx) * 0.55 * saturate((q.y - 0.42) / 0.1) * saturate((0.95 - q.y) / 0.3));
    }
    // The burst of light as it lands.
    float flash = pow(sin(PI * saturate((grow - 0.4) / 0.6)), 2.0);
    colour += float3(1.0, 0.95, 0.85) * 0.35 * flash * exp(-dot(c, c) * 1.2);
    return colour;
}

// ---------------------------------------------------------------------------------------------

// The colour of a sign's face. `q` is the point in characters from the top-left corner of the
// words and `d` how far it is from their outline (negative inside); `edge` is how far in from
// the sign's edge it is and `px` a screen pixel, both in characters; `hum` how brightly it
// burns; `picture` what a screen shows behind its word.
float3 SignFace(Sign s, float2 q, float d, float edge, float px, float hum, float3 picture)
{
    // How well the characters can be drawn at this size: filled from about 6 pixels high, as
    // tube round their strokes from about 16. Smaller than that they fade to an even glow, so
    // they do not shimmer.
    float shown = saturate(((1.0 / px) - 6.0) / 4.0);
    float tubed = saturate(((1.0 / px) - 16.0) / 8.0);
    float3 panel = float3(0.024, 0.018, 0.034);
    float3 face;
    [branch]
    if (s.look == 7)
    {
        // An advert poster: its picture, and its lettering in the brand's ink with a fine dark
        // edge, so it reads on any part of the picture.
        float letters = Inside(-d, px) * shown;
        float edging = Inside(0.025 - d, px) * (1.0 - letters) * shown;
        face = lerp(picture, float3(0.03, 0.02, 0.03), edging * 0.6);
        face = lerp(face, s.trim, letters);
    }
    else if (s.look == 5)
    {
        // A video screen: its picture, the word over it in white, a thin dark bezel and a
        // faint grid of the screen's own pixels.
        float letters = Inside(-d, px) * shown;
        face = lerp(picture * 0.82, float3(0.85, 0.85, 0.90), letters);
        face *= lerp(0.30, 1.0, Inside(edge - 0.04, px));
    }
    else if (s.look == 6)
    {
        // An LED ticker: dots of light on black where the characters are.
        float lit = Inside(-d, px);
        float bulb = Inside(0.36 - length(frac(q * 8.0) - 0.5), px * 8.0);
        bulb = lerp(0.8, bulb, saturate(((1.0 / (px * 8.0)) - 2.5) / 1.0));
        face = float3(0.012, 0.010, 0.014) + (s.colour * lerp(0.25, (bulb * lit) + (0.05 * bulb), shown));
    }
    else if (s.look == 2)
    {
        // Ink on a box lit from inside, with a thin dark frame.
        float ink = lerp(0.30, Inside(-d, px), shown);
        face = lerp(s.colour * 0.58, s.trim, ink);
        face *= lerp(0.35, 1.0, Inside(edge - 0.05, px));
    }
    else if (s.look == 3)
    {
        // Dots of light where the characters are, with a dashed tube round the edge. Each point
        // takes its light from the character itself, so the words keep their shape; eight dots to
        // a character (fourteen on a big sign) are laid over them, smoothing into even light once
        // they are under about 3.5 pixels apart.
        float lit = Inside(-d, px);
        float dots = (s.size > 0.09) ? 14.0 : 8.0;
        float bulb = Inside(0.36 - length(frac(q * dots) - 0.5), px * dots);
        bulb = lerp(0.8, bulb, saturate(((1.0 / (px * dots)) - 2.5) / 1.0));
        float letters = lerp(0.30, (bulb * lit) + (0.07 * bulb * (1.0 - lit)), shown);
        float dash = step(0.45, frac((q.x + q.y) * 6.0));
        float border = Inside(0.028 - abs(edge - 0.08), px) * lerp(0.6, dash, shown);
        face = panel + (s.colour * letters) + (s.trim * border * 0.85);
    }
    else
    {
        // Neon: either a tube round each stroke or the strokes filled with light, brightest
        // down their middles, with a soft glow round them and a tube round the sign's edge.
        float filled = Inside(-d, px) * (0.72 + (0.28 * saturate(-d / 0.05)));
        float tube = Inside(0.022 - abs(d + 0.014), px) + (0.30 * exp(-abs(d + 0.014) / 0.016));
        float strokes = (s.look == 0) ? lerp(filled, tube, tubed) : filled;
        float halo = 0.30 * exp(-max(d, 0.0) / 0.02) * (1.0 - Inside(-d, px));
        float letters = lerp(0.34, saturate(strokes + halo), shown);
        float border = Inside(0.030 - abs(edge - 0.085), px) * shown;
        face = panel + (s.colour * letters) + (s.trim * border * 0.85);
    }
    return face * hum * NEON;
}

// The colour of sign `s` at point `p`, where a screen pixel is `aa` long across and up. This
// is where words are picked for the signs that leave it until they are drawn, and where the
// one lookup of the characters' table is made.
float3 SignColour(Sign s, float2 p, float2 aa, float clock)
{
    Sign t = s;
    // Where the words go, and how many characters fit across (or down) it.
    float4 area = s.box;
    float edge = BoxDepth(p, s.box);
    float framed = 0.0;
    float3 picture = float3(0.0, 0.0, 0.0);
    float most = 0.0;
    [branch]
    if (s.look == 4)
    {
        // A stack of small signs, one to a tenant, each a word of one or two characters,
        // hung one above the other on a dark frame.
        float span = ((2.0 + (2.0 * PAD)) + 0.35) * s.size;
        float k = floor((s.box.w - p.y) / span);
        float tenant = (s.id * 16.0) + k;
        Dress(t, tenant);
        t.vertical = 1.0;
        area = float4(s.box.x, s.box.y, s.box.w - (k * span) - ((2.0 + (2.0 * PAD)) * s.size), s.box.w - (k * span));
        framed = step(p.y, area.z);
        edge = min(edge, BoxDepth(p, area));
        most = 2.0;
    }
    else if (s.look == 5)
    {
        // A screen: the picture in a square at the left (or in the middle of a narrow screen),
        // and the word beside it (or along the bottom).
        float width = s.box.y - s.box.x;
        float height = s.box.w - s.box.z;
        bool wide = width > (1.3 * height);
        float2 centre = wide ? float2(s.box.x + (0.5 * height), s.box.z + (0.5 * height)) : float2((s.box.x + s.box.y) * 0.5, s.box.z + (0.62 * height));
        float half_ = wide ? (0.47 * height) : (0.34 * height);
        float2 c = (p - centre) / half_;
        picture = Picture(s, c, max(aa.x, aa.y) / half_, clock);
        t.size = wide ? (0.30 * height) : (0.20 * height);
        t.vertical = 0.0;
        area = wide ? float4(s.box.x + (0.95 * height), s.box.y - (0.04 * height), s.box.z, s.box.w) : float4(s.box.x, s.box.y, s.box.z + (0.02 * height), s.box.z + (0.30 * height));
        most = floor(((area.y - area.x) / t.size) - (2.0 * PAD));
    }
    else if (s.look != 6)
    {
        most = floor(((area.y - area.x) / t.size) - (2.0 * PAD));
    }
    if ((t.word >= WORD_COUNT) && (s.look != 6) && (s.look != 7) && (most >= 1.0))
    {
        t.word = PickWord(t.id, 800u, most);
    }
    // The run of text to draw: the sign's word, or on a poster the brand's name across the top
    // and its slogan along the bottom.
    uint2 run = (t.word < WORD_COUNT) ? WORDS[t.word] : uint2(0, 0);
    [branch]
    if (s.look == 7)
    {
        float width = s.box.y - s.box.x;
        float height = s.box.w - s.box.z;
        uint brand = Brand(s);
        // A tall poster has its product in the middle, the brand's name across the top and the
        // slogan along the bottom; a wide one has its product at the left and the name over the
        // slogan beside it.
        bool wide = width > (1.25 * height);
        float2 centre = wide ? float2(s.box.x + (0.55 * height), s.box.z + (0.50 * height)) : float2((s.box.x + s.box.y) * 0.5, s.box.z + (0.45 * height));
        float half_ = wide ? (0.42 * height) : min(0.52 * width, 0.33 * height);
        picture = Advert(brand, (p - centre) / half_, max(aa.x, aa.y) / half_, clock, s.colour, Pick(s.id, 701u));
        bool upper = wide ? (p.y > (s.box.z + (0.42 * height))) : (p.y > (s.box.z + (0.70 * height)));
        float left = s.box.x + (1.10 * height);
        area = wide ? (upper ? float4(left, s.box.y - (0.04 * width), s.box.z + (0.42 * height), s.box.w - (0.08 * height))
                             : float4(left, s.box.y - (0.04 * width), s.box.z + (0.08 * height), s.box.z + (0.38 * height)))
                    : (upper ? float4(s.box.x + (0.05 * width), s.box.y - (0.05 * width), s.box.w - (0.25 * height), s.box.w - (0.03 * height))
                             : float4(s.box.x + (0.10 * width), s.box.y - (0.10 * width), s.box.z + (0.035 * height), s.box.z + (0.135 * height)));
        // (Both sides of ?: are read, so the brands' table is never read past its end.)
        uint slogan = (Pick(s.id, 702u) < 0.6) ? WORD_NEW : WORD_LIMITED;
        run = upper ? ((brand < BRAND_COUNT) ? BRAND_WORDS[min(brand, BRAND_COUNT - 1)] : WORDS[WORD_RAMEN]) : WORDS[slogan];
        // A brand's name fills its strip edge to edge; a word keeps its margin.
        float framing = (upper && (brand < BRAND_COUNT)) ? 0.0 : (2.0 * PAD);
        t.size = min((area.w - area.z) / (1.0 + framing), (area.y - area.x) / (run.y + framing));
        t.vertical = 0.0;
        t.trim = upper ? s.trim : float3(1.0, 1.0, 1.0);
    }
    // The point against the words: in characters from the top-left corner of the first.
    float2 extent = float2(area.y - area.x, area.w - area.z) / t.size;
    float2 from_top = float2(p.x - area.x, area.w - p.y) / t.size;
    float2 q = from_top;
    float2 uv = float2(0.0, 0.0);
    uint g = TICKER_BLANK;
    if (s.look == 6)
    {
        q.y -= (extent.y - 1.0) * 0.5;
        g = TickerChar(q, (Ease(clock) * 9.0) + (Pick(t.id, 97u) * TICKER_LENGTH), uv);
    }
    else if (run.y > 0)
    {
        float n = run.y;
        float2 words = (t.vertical > 0.5) ? float2(1.0, n) : float2(n, 1.0);
        q = from_top - ((extent - words) * 0.5);
        g = TextChar(q, run, t.vertical, uv);
    }
    float d = 1.0;
#if !ENV
    [branch]
    if (g != TICKER_BLANK)
    {
        d = Glyph(g, uv);
    }
#endif
    // Small signs buzz and drop out now and then; the big ones and the screens only flicker,
    // so the street does not darken when one of them buzzes.
    float hum = Hum(t.id, clock);
    hum = ((s.look == 5) || (s.look == 7) || (s.size > 0.09)) ? lerp(1.0, hum, 0.25) : hum;
    float3 face = SignFace(t, q, d, edge / t.size, max(aa.x, aa.y) / t.size, hum, picture);
    if (s.look == 7)
    {
        // The poster is lit by floodlights along its foot, brighter low down, and sits in a thin
        // dark frame.
        float rise = (p.y - s.box.z) / (s.box.w - s.box.z);
        face *= 0.62 + (0.40 * exp(-rise / 0.22));
        face = lerp(face, float3(0.05, 0.05, 0.06), saturate(0.5 + ((0.006 - edge) / max(aa.x, aa.y))));
    }
    return lerp(face, float3(0.025, 0.025, 0.03), framed);
}

// The sign a ring shows at a point, kept so its face can be drawn once after all the rings: the
// characters' table is costly to build into the shader, and far more so inside the rings' loop.
struct SignHit
{
    Sign sign;
    float2 p;
    float2 aa;
    float weight;   // how much of the picture it makes up
};

// The light a sign spills into the wet air round it, with the long thin sideways flare an
// anamorphic lens draws from bright lights.
float3 SignGlow(Sign s, float2 p, float clock)
{
    // A pale lit box or a screen is a large bright area, so the light it throws is held down
    // more than a neon tube's.
    float3 light = (s.look == 2) ? (s.colour * 0.55) : (lerp(s.colour, s.trim, 0.3) * ((s.look == 5) ? 0.6 : 1.0));
    if (s.look == 4)
    {
        light = float3(0.85, 0.45, 0.80);
    }
    if (s.look == 7)
    {
        light = s.colour * 0.7;
    }
    float2 outside = max(max(s.box.xz - p, p - s.box.yw), 0.0);
    float3 glow = light * 0.15 * exp(-length(outside) / 0.03);
    glow += lerp(light, float3(0.35, 0.65, 1.00), 0.6) * 0.06 * exp(-outside.y / 0.004) * exp(-outside.x / 0.045);
    float hum = Hum(s.id, clock);
    hum = ((s.look == 5) || (s.look == 7) || (s.size > 0.09)) ? lerp(1.0, hum, 0.25) : hum;
    return glow * hum * step(s.box.z, s.box.w);
}

// A building's big signs.
struct Signs
{
    Sign tower;
    Sign side;
    Sign roof;
};

Signs Layout(Block b, float width)
{
    Signs s;
    s.tower = TowerSign(b);
    s.side = SideSign(b, width, s.tower);
    s.roof = RoofSign(b, width, s.tower);
    return s;
}

// The colour a building's walls are: dark metal for offices and billboard towers, concrete for
// flats and old blocks, pale tile for the bar buildings, each a little different.
float3 WallColour(Block b)
{
    float3 wall = (b.kind == 0) ? float3(0.055, 0.060, 0.078)
        : (b.kind == 1) ? float3(0.080, 0.078, 0.082)
        : (b.kind == 2) ? float3(0.100, 0.088, 0.085)
        : (b.kind == 3) ? float3(0.075, 0.070, 0.066)
        : float3(0.040, 0.040, 0.050);
    return wall * (0.80 + (0.40 * Pick(b.id, 10u)));
}

// One window bay of a near building. `f` is the point within the bay, 0 to 1 across and up
// the floor; `size` is the bay's size; `cell` which bay and floor it is; `aa` a screen pixel.
float3 Bay(Block b, float3 wall, float2 f, float2 size, float2 cell, float2 aa, float clock)
{
    // How finely the bay can be drawn: fine lines like blinds and slats fade out as it shrinks.
    float fine = saturate((min(size.x / aa.x, size.y / aa.y) - 12.0) / 12.0);
    float id = b.id;
    float window_id = floor(Pick3(cell, id, 11u) * 1048576.0);
    float3 colour = wall;
    // How far from the bay's sides, in screen pixels.
    float side = min(f.x, 1.0 - f.x) * size.x / aa.x;
    [branch]
    if ((b.kind == 0) || (b.kind == 4))
    {
        // Offices, and the billboard towers under their panels: a curtain wall of glass in bands, with a dark spandrel at each floor and thin
        // mullions. Whole floors are lit in cool white, and some runs of bays.
        float floor_lit = step(Pick3(float2(0.0, cell.y), id, 12u), 0.55);
        float run = step(Pick3(float2(floor(cell.x / 3.0), cell.y), id, 13u), 0.22);
        float lit = Switching(window_id, clock, abs(floor_lit - run));
        float glass = saturate(0.5 + ((f.y - 0.24) * size.y / aa.y));
        float3 light = lerp(float3(0.62, 0.80, 1.00), float3(0.95, 0.97, 1.00), Pick3(float2(0.0, cell.y), id, 14u));
        light *= 0.34 + (0.24 * Pick3(cell, id, 15u));
        // The ceiling's light strip and the desks low down.
        light *= 0.80 + (0.55 * exp(-pow((f.y - 0.95) / 0.035, 2.0)));
        light *= 1.0 - (0.40 * saturate(0.5 + ((0.36 - f.y) * size.y / aa.y)) * step(0.5, frac((f.x * 2.0) + Pick3(cell, id, 16u))));
        // Unlit glass gives back the night sky, with a faint sheen slanting across it.
        float3 night = lerp(float3(0.030, 0.028, 0.065), float3(0.11, 0.07, 0.18), saturate((f.y - 0.24) / 0.76));
        night += 0.05 * fine * smoothstep(0.25, 0.0, abs(frac(f.x + (0.4 * f.y) + Pick3(cell, id, 17u)) - 0.5));
        float3 pane = lerp(night, light, lit);
        colour = lerp((wall * 0.55) + float3(0.012, 0.012, 0.018), pane, glass);
        colour = lerp(colour, wall * 0.35, saturate(1.5 - side));
    }
    else if (b.kind == 1)
    {
        float lit = Switching(window_id, clock, step(Pick3(cell, id, 18u), 0.45));
        float3 light = lerp(float3(1.00, 0.66, 0.36), float3(1.00, 0.84, 0.60), Pick3(cell, id, 19u)) * (0.45 + (0.35 * Pick3(cell, id, 21u)));
        if (Pick(id, 8u) < 0.5)
        {
            // Flats seen from the balcony side: a slab edge, a frosted railing panel, and the
            // windows set back behind it, curtained when lit. Some balconies keep an air
            // conditioner by the railing.
            float slab = saturate(0.5 + ((0.10 - f.y) * size.y / aa.y));
            float railing = saturate(0.5 + ((0.42 - f.y) * size.y / aa.y)) * (1.0 - slab);
            float folds = 0.80 + (0.20 * fine * sin(f.x * size.x * 420.0));
            float3 window = lerp(float3(0.020, 0.020, 0.035), light * folds, lit);
            float3 panel = lerp(float3(0.11, 0.115, 0.13), light * 0.32, lit * 0.6) * (0.92 + (0.08 * fine * sin(f.x * size.x * 900.0)));
            colour = window;
            colour = lerp(colour, panel, railing);
            colour = lerp(colour, wall * 1.7, slab);
            float unit = step(Pick3(cell, id, 22u), 0.45);
            float box = BoxCover(f * size, float4(0.66 * size.x, 0.92 * size.x, 0.10 * size.y, 0.31 * size.y), aa) * unit;
            colour = lerp(colour, float3(0.26, 0.26, 0.28) * (0.6 + (0.4 * f.y)), box);
            // The wall between neighbours' balconies.
            colour = lerp(colour, wall * 1.2, saturate(2.0 - side));
        }
        else
        {
            // Flats seen from the corridor side: an open walkway on every floor behind a solid
            // parapet, its ceiling lamps lighting the doors.
            float parapet = saturate(0.5 + ((0.40 - f.y) * size.y / aa.y));
            float slab = saturate(0.5 + ((0.07 - f.y) * size.y / aa.y));
            float lamp_on = Switching(window_id + 3.0, clock, step(Pick3(cell, id, 23u), 0.90));
            float2 at = (f - float2(0.5, 0.94)) * size;
            float lamp = BoxCover(at, float4(-0.007, 0.007, -0.002, 0.002), aa) * lamp_on;
            float pool = exp(-pow(at.x / 0.03, 2.0)) * saturate((f.y - 0.35) / 0.6) * lamp_on;
            float door = BoxCover(f * size, float4(0.30 * size.x, 0.56 * size.x, 0.40 * size.y, 0.88 * size.y), aa);
            float3 corridor = lerp(float3(0.025, 0.026, 0.030), float3(0.10, 0.095, 0.085), door);
            corridor += pool * float3(0.30, 0.34, 0.36) * lerp(0.6, 1.2, door);
            corridor = lerp(corridor, float3(0.90, 0.97, 1.00), lamp);
            colour = lerp(corridor, wall * (1.15 + (0.25 * pool)), parapet);
            colour = lerp(colour, wall * 1.5, slab);
        }
    }
    else if (b.kind == 2)
    {
        // A bar building: pale tile, and on each floor two windows glowing softly the colour of
        // whatever bar is up there, a small lit panel between them, and a dark band of wall
        // between floors. A few floors are shut.
        float tiles = 0.92 + (0.08 * fine * step(0.08, frac(f.x * size.x / 0.012)) * step(0.08, frac(f.y * size.y / 0.012)));
        colour = wall * tiles;
        float open = Switching(window_id, clock, step(Pick3(float2(0.0, cell.y), id, 24u), 0.72));
        float3 bar = lerp(Neon(Pick3(float2(0.0, cell.y), id, 25u)), float3(1.0, 0.72, 0.48), 0.55) * 0.34;
        float blinds = 0.80 + (0.20 * fine * step(0.5, frac(f.y * size.y / 0.006)));
        float half_ = step(0.5, f.x);
        float x = frac(f.x * 2.0);
        float window = BoxCover(float2(x * size.x * 0.5, f.y * size.y), float4(0.12 * size.x * 0.5, 0.80 * size.x * 0.5, 0.26 * size.y, 0.80 * size.y), aa);
        float3 room = lerp(float3(0.02, 0.02, 0.03), bar * blinds * (0.8 + (0.4 * Pick3(cell + float2(half_, 0.0), id, 26u))), open);
        colour = lerp(colour, room, window);
        float panel = BoxCover(f * size, float4(0.44 * size.x, 0.56 * size.x, 0.36 * size.y, 0.70 * size.y), aa) * step(Pick3(float2(0.0, cell.y), id, 27u), 0.6);
        colour = lerp(colour, lerp(float3(0.03, 0.03, 0.04), Neon(Pick3(float2(1.0, cell.y), id, 28u)) * 0.55, open), panel);
        colour = lerp(colour, wall * 0.45, saturate(0.5 + ((0.10 - f.y) * size.y / aa.y)));
    }
    else
    {
        // An old concrete block: small windows with pale frames, blinds in the lit ones, an air
        // conditioner under some and a drainpipe down every third bay.
        float lit = Switching(window_id, clock, step(Pick3(cell, id, 26u), 0.38));
        float3 light = lerp(float3(1.00, 0.72, 0.42), float3(0.80, 0.92, 1.00), step(0.6, Pick3(cell, id, 27u))) * (0.45 + (0.35 * Pick3(cell, id, 28u)));
        float blinds = 0.78 + (0.22 * fine * step(0.45, frac(f.y * size.y / 0.005)));
        float4 frame_box = float4(0.20 * size.x, 0.80 * size.x, 0.30 * size.y, 0.88 * size.y);
        float frame = BoxCover(f * size, frame_box, aa);
        float pane = BoxCover(f * size, frame_box + float4(0.005, -0.005, 0.005, -0.005), aa);
        colour = lerp(colour, wall * 1.6, frame);
        colour = lerp(colour, lerp(float3(0.02, 0.02, 0.03), light * blinds, lit), pane);
        float unit = step(Pick3(cell, id, 29u), 0.40);
        float box = BoxCover(f * size, float4(0.26 * size.x, 0.56 * size.x, 0.06 * size.y, 0.25 * size.y), aa) * unit;
        float fan = saturate(1.0 - length(((f * size) - float2(0.41 * size.x, 0.155 * size.y)) / (0.07 * size.y)));
        colour = lerp(colour, float3(0.28, 0.28, 0.29) * (1.0 - (0.5 * step(0.15, fan) * fine)), box);
        float pipe = BoxCover(f * size, float4(-0.003, 0.003, -1.0, 2.0), aa) * step(fmod(cell.x, 3.0), 0.5);
        colour = lerp(colour, wall * 0.5, pipe);
    }
    return colour;
}

// The ground floor: shops side by side behind pillars, lit inside, some with their shutters
// part way down and some with a row of red paper lanterns under the sign; a pair of glowing
// drink machines at the corner of some buildings.
float3 Shopfront(Block b, float3 wall, float2 f, float w, float2 aa, float clock)
{
    float shops = Shops(b);
    float span = w / shops;
    float k = floor(f.x / span);
    float x = f.x - (k * span);
    float shop = (b.id * 4.0) + k;
    float fine = saturate(((0.01 / aa.y) - 1.0) / 2.0);
    float3 colour = wall * 0.6;
    float3 inside = lerp(float3(1.00, 0.78, 0.52), float3(0.88, 0.95, 1.00), step(0.5, Pick(shop, 22u))) * (0.22 + (0.22 * Pick(shop, 23u)));
    inside *= 0.80 + (0.20 * fine * step(0.5, frac(f.y / 0.012)));
    float glass = BoxCover(float2(x, f.y), float4(0.008, span - 0.008, -1.0, GROUND - 0.030), aa);
    float shutter = step(Pick(shop, 24u), 0.22);
    float down = (GROUND - 0.030) - ((0.3 + (0.7 * Pick(shop, 27u))) * (GROUND - 0.030));
    float3 front = inside * (0.85 + (0.15 * fine * step(0.06, frac(x / 0.035))));
    float3 metal = float3(0.07, 0.07, 0.08) * (0.85 + (0.15 * fine * step(0.5, frac(f.y / 0.004))));
    front = lerp(front, metal, shutter * step(down, f.y));
    colour = lerp(colour, front, glass);
    // Red paper lanterns hung in a row under the shop's sign.
    [branch]
    if (Pick(shop, 25u) < 0.35)
    {
        float2 r = float2((frac(x / 0.028) - 0.5) * 0.028 / 0.009, (f.y - (GROUND - 0.042)) / 0.012);
        float round_ = length(r);
        float lantern = saturate(0.5 + ((1.0 - round_) * 0.010 / aa.y)) * step(0.01, x) * step(x, span - 0.01);
        float bands = 0.80 + (0.20 * fine * cos(r.y * 9.0));
        colour = lerp(colour, float3(1.00, 0.30, 0.16) * bands * (1.0 - (0.35 * round_ * round_)), lantern);
    }
    // Drink machines at the corner.
    [branch]
    if (Pick(b.id, 26u) < 0.4)
    {
        float machine = BoxCover(f, float4(w - 0.034, w - 0.008, 0.0, 0.050), aa);
        float3 face = float3(0.80, 0.88, 1.00) * 0.55;
        face = lerp(face, Neon(Pick3(floor(f / 0.006), b.id, 31u)) * 0.8, fine * step(0.030, f.y) * step(f.y, 0.044) * step(0.5, frac(f.x / 0.006)));
        face = lerp(face, float3(0.10, 0.10, 0.12), step(f.y, 0.018));
        colour = lerp(colour, face, machine);
    }
    return colour;
}

// What stands on a roof: a railing round the edge, a water tank on legs, boxes of machinery,
// and on the taller buildings a mast with a red light. All dark against the glow of the sky.
float4 Rooftop(Block b, float l, float r, float2 p, float2 aa, float clock)
{
    float top = b.top;
    float w = r - l;
    float cover = BoxCover(p, float4(l, r, top + 0.0110, top + 0.0135), aa);
    float post = frac((p.x - l) / 0.025) * 0.025;
    cover = max(cover, BoxCover(float2(post, p.y), float4(0.0, 0.0016, top, top + 0.0135), aa) * step(l, p.x) * step(p.x, r));
    [branch]
    if (Pick(b.id, 30u) < 0.55)
    {
        float tx = l + (w * (0.08 + (0.55 * Pick(b.id, 31u))));
        float tw = 0.045 + (0.025 * Pick(b.id, 32u));
        cover = max(cover, BoxCover(p, float4(tx, tx + tw, top + 0.020, top + 0.056), aa));
        float cap = length((p - float2(tx + (tw * 0.5), top + 0.056)) / float2(tw * 0.5, 0.009));
        cover = max(cover, saturate(0.5 + ((1.0 - cap) * 0.009 / aa.y)));
        cover = max(cover, BoxCover(p, float4(tx + 0.004, tx + 0.007, top, top + 0.021), aa));
        cover = max(cover, BoxCover(p, float4(tx + tw - 0.007, tx + tw - 0.004, top, top + 0.021), aa));
    }
    [unroll]
    for (uint k = 0; k < 2; k++)
    {
        float ux = l + (w * (0.05 + (0.75 * Pick(b.id, 33u + k))));
        float uw = 0.030 + (0.030 * Pick(b.id, 35u + k));
        float uh = 0.012 + (0.016 * Pick(b.id, 37u + k));
        cover = max(cover, BoxCover(p, float4(ux, ux + uw, top, top + uh), aa));
    }
    float3 colour = float3(0.020, 0.020, 0.032);
    [branch]
    if (top > 0.75)
    {
        float mx = l + (w * (0.30 + (0.40 * Pick(b.id, 39u))));
        float mh = 0.10 + (0.08 * Pick(b.id, 40u));
        cover = max(cover, BoxCover(p, float4(mx - 0.0013, mx + 0.0013, top, top + mh), aa));
        float2 tip = (p - float2(mx, top + mh)) / max(aa, 0.003);
        float lamp = saturate(exp(-dot(tip, tip) / 2.5) * 1.5) * Beacon(b.id, clock);
        colour = lerp(colour, float3(1.0, 0.16, 0.12), lamp);
        cover = max(cover, lamp);
    }
    return float4(colour, cover);
}

// Fire escapes and air conditioners on the narrow old buildings: an iron stair zigzagging up one
// side floor by floor, and outdoor units with their pipes clinging to the other. A bar
// building's stack of signs hangs on one side, so its stair goes on the other. `light` is the
// signs' light falling on them.
float3 Clutter(Block b, float3 colour, float3 wall, float2 f, float w, float storey, float2 aa, float3 light)
{
    float stack_left = (Pick(b.id, 51u) < 0.5) ? 1.0 : 0.0;
    float stair_left = (b.kind == 2) ? (1.0 - stack_left) : step(Pick(b.id, 120u), 0.5);
    float level = floor(storey);
    float up = frac(storey) * b.storey;
    [branch]
    if (Pick(b.id, 121u) < 0.75)
    {
        float x = (stair_left > 0.5) ? f.x : (w - f.x);
        float2 q = float2(x, up);
        float landing = BoxCover(q, float4(0.0, 0.070, 0.0, 0.0035), aa);
        float rail = BoxCover(q, float4(0.0, 0.070, 0.0235, 0.026), aa);
        float post = BoxCover(float2(frac(x / 0.0233) * 0.0233, up), float4(0.0, 0.0016, 0.0, 0.026), aa) * step(x, 0.071);
        float t = up / b.storey;
        float run = (fmod(level, 2.0) < 0.5) ? lerp(0.010, 0.062, t) : lerp(0.062, 0.010, t);
        float stair = saturate(0.5 + ((0.0022 - abs(x - run)) / aa.x)) * step(x, 0.07);
        float metal = max(max(landing, rail), max(post, stair));
        colour = lerp(colour, float3(0.022, 0.022, 0.028) + (light * 0.4), metal);
    }
    [branch]
    if (Pick(b.id, 122u) < 0.8)
    {
        float x = (stair_left > 0.5) ? (w - f.x) : f.x;
        float2 q = float2(x, up);
        float unit = step(Pick3(float2(level, 1.0), b.id, 123u), 0.75);
        float shift = 0.010 * Pick3(float2(level, 2.0), b.id, 124u);
        float box = BoxCover(q, float4(0.006 + shift, 0.040 + shift, 0.006, 0.030), aa) * unit;
        float fan = length(q - float2(0.020 + shift, 0.018)) - 0.008;
        float3 ac = (float3(0.26, 0.26, 0.27) * (0.7 + (0.3 * saturate(up / 0.03)))) + (light * 0.5);
        ac = lerp(ac, ac * 0.35, Fill(fan, aa.y));
        colour = lerp(colour, ac, box);
        float pipes = max(BoxCover(q, float4(0.046, 0.049, -1.0, 2.0), aa), BoxCover(q, float4(0.054, 0.056, -1.0, 2.0), aa));
        colour = lerp(colour, wall * 0.55, pipes);
    }
    return colour;
}

// The purple of the city's haze, which the mist low in the streets takes on.
static const float3 HAZE = float3(0.27, 0.10, 0.25);

// One ring of near buildings: what it shows at a direction. The ring is `D` away, with `cells`
// places round it each holding a building or an open street, and buildings `low` to `high`
// tall. Returns the colour and how much it covers; the light its signs spill into the air is
// added to `glow`. Where a sign covers the point, the colour is left out in proportion and the
// sign is handed back in `hit`, to be drawn after all the rings. Below the street the ring is
// seen again, upside down in the wet asphalt.
float4 Facade(float phi, float e, float pixel, float clock, float D, float cells, float low, float high, float ring, inout float3 glow, inout SignHit hit)
{
    float ce = max(cos(e), 0.05);
    float height = (D * tan(clamp(e, -1.3, 1.45))) + EYE;
    // The reflection in the street: blurred and rippled, and dimmer the more steeply the
    // street is looked down at.
    float mirrored = step(height, 0.0);
    float h = abs(height);
    float shine = lerp(1.0, 0.10 + (0.42 * exp(e / 0.10)), mirrored);
    float angle = TAU / cells;
    float at = (phi + PI) / angle;
    float place = floor(at);
    float width = angle * D;
    float2 p = float2(frac(at) * width, h);
    // A screen pixel on the facade, across and up: looking up, the floors crowd together, so a
    // pixel covers more of the wall up than across.
    float2 aa = float2(pixel * D / ce, pixel * D / (ce * ce));
    if (mirrored > 0.5)
    {
        p.x += 0.002 * sin((h * 150.0) + (Ease(clock) * TAU * 2.0) + (Noise2(float2(p.x * 40.0, h * 8.0)) * 6.0));
        aa *= float2(3.0, 6.0);
    }
    float id = fmod(place, cells) + 1000.0 + (ring * 97.0);
    // Above the tallest building the ring could have, and the signs on its roofs, there is
    // nothing of it, so none of it is worked out.
    [branch]
    if (h > high + 0.35)
    {
        return float4(0.0, 0.0, 0.0, 0.0);
    }

    // This building and its signs, and the light its signs spill into the air; the neighbours'
    // too where the point is near them. One loop, so the layout is built into the shader once.
    Block b = (Block)0;
    Signs signs = (Signs)0;
    float3 local = float3(0.0, 0.0, 0.0);
    [loop]
    for (int k = -1; k <= 1; k++)
    {
        if (((k < 0) && (p.x > 0.12)) || ((k > 0) && (p.x < width - 0.12)))
        {
            continue;
        }
        Block nb = NearBlock(fmod(place + cells + k, cells) + 1000.0 + (ring * 97.0), width, low, high);
        Signs ns = Layout(nb, width);
        float2 q = p - float2(k * width, 0.0);
        local += SignGlow(ns.tower, q, clock) + SignGlow(ns.side, q, clock) + SignGlow(ns.roof, q, clock);
        if (k == 0)
        {
            b = nb;
            signs = ns;
        }
    }
    // The panels on this band of floors, and on the bands above and below, light the air too.
    float band = BandHeight(b);
    float ceiling = Shown(signs.tower) ? (signs.tower.box.z - 0.01) : 100.0;
    Sign band_sign = Blank();
    [loop]
    for (int j = -1; j <= 1; j++)
    {
        Sign s = BandSign(b, h + (j * band), ceiling, signs.side);
        local += SignGlow(s, p, clock);
        if (j == 0)
        {
            band_sign = s;
        }
    }
    local *= NEON;
    glow += local * shine;

    float4 result = float4(0.0, 0.0, 0.0, 0.0);
    [branch]
    if ((b.present > 0.5) && (h < b.top + 0.30) && (p.x > b.l - 0.10) && (p.x < b.r + 0.10))
    {
        float l = b.l;
        float r = b.r;
        if (Setback(b) && (h > b.top * 0.82))
        {
            float cut = (r - l) * 0.18;
            l += cut;
            r -= cut;
        }
        float body = BoxCover(p, float4(l, r, -1.0, b.top), aa);
        [branch]
        if (body > 0.0)
        {
            float2 f = float2(p.x - l, h);
            float w = r - l;
            // The wall: a little uneven, streaked by the rain, lit from the street below and by
            // the signs round it, and darker into its corners.
            float3 wall = WallColour(b);
            wall *= 0.85 + (0.30 * Noise2((f * float2(9.0, 5.0)) + (id * 0.37)));
            wall *= 1.0 - (0.30 * smoothstep(0.5, 0.85, Noise2(float2(f.x * 60.0, f.y * 3.0) + id)));
            wall *= 1.0 + (1.4 * exp(-h / 0.12));
            wall *= 0.55 + (0.45 * smoothstep(0.0, 0.05, min(f.x, w - f.x)));
            wall += local * (((b.kind == 0) || (b.kind == 4)) ? 0.45 : 0.30);
            float3 colour = wall;
            float floors = floor((b.top - GROUND - 0.02) / b.storey);
            float storey = (h - GROUND) / b.storey;
#if ENV
            // Reflections are small and soft: the floors show as their average light, which
            // keeps those shaders quick to build.
            colour = (h < GROUND) ? ((wall * 1.4) + float3(0.10, 0.08, 0.06)) : ((storey < floors) ? lerp(wall, float3(0.42, 0.38, 0.34), 0.22) : wall);
#else
            [branch]
            if (h < GROUND)
            {
                colour = Shopfront(b, wall, f, w, aa, clock);
            }
            else if (storey < floors)
            {
                float bay = w / b.bays;
                float2 g = float2(f.x / bay, storey);
                colour = Bay(b, wall, frac(g), float2(bay, b.storey), floor(g), aa, clock);
                [branch]
                if ((b.kind == 2) || (b.kind == 3))
                {
                    colour = Clutter(b, colour, wall, f, w, storey, aa, local);
                }
            }
#endif
            // A band of neon round the crown of some towers, and a strip of light down one
            // corner of others.
            float glassy = ((b.kind == 0) || (b.kind == 4)) ? 1.0 : 0.0;
            colour = lerp(colour, Neon(Pick(id, 36u)) * 0.85 * NEON, BoxCover(f, float4(0.0, w, b.top - 0.013, b.top - 0.006), aa) * glassy * step(Pick(id, 35u), 0.45));
            colour = lerp(colour, Neon(Pick(id, 38u)) * 0.8 * NEON, BoxCover(f, float4(0.0, 0.004, GROUND, b.top - 0.02), aa) * glassy * step(Pick(id, 37u), 0.30));
            // Mist low in the street, catching the neon.
            colour = lerp(colour, (HAZE * 0.55) + (local * 2.0), 0.22 * exp(-h / 0.20));
            result = float4(colour, body);
        }
#if !ENV
        float4 roof = Rooftop(b, l, r, p, aa, clock);
        result.rgb = lerp(result.rgb, roof.rgb, roof.a);
        result.a = max(result.a, roof.a);
#endif
        // The roof board's legs.
        Sign roof_sign = signs.roof;
        float legs = BoxCover(p, float4(roof_sign.box.x + 0.02, roof_sign.box.x + 0.024, b.top, roof_sign.box.z), aa);
        legs = max(legs, BoxCover(p, float4(roof_sign.box.y - 0.024, roof_sign.box.y - 0.02, b.top, roof_sign.box.z), aa));
        legs *= step(roof_sign.box.z, roof_sign.box.w);
        result.rgb = lerp(result.rgb, float3(0.03, 0.03, 0.04), legs);
        result.a = max(result.a, legs);
        // The signs never overlap, so only the one the point is deepest inside counts.
        Sign sign = signs.tower;
        if (BoxDepth(p, signs.side.box) > BoxDepth(p, sign.box))
        {
            sign = signs.side;
        }
        if (BoxDepth(p, band_sign.box) > BoxDepth(p, sign.box))
        {
            sign = band_sign;
        }
        if (BoxDepth(p, roof_sign.box) > BoxDepth(p, sign.box))
        {
            sign = roof_sign;
        }
        Sign shop_sign = ShopSign(b, p.x);
        if (BoxDepth(p, shop_sign.box) > BoxDepth(p, sign.box))
        {
            sign = shop_sign;
        }
        float cover = BoxCover(p, sign.box, aa);
#if ENV
        result.rgb = lerp(result.rgb, lerp(sign.colour, sign.trim, 0.3) * 0.45 * NEON, cover);
        result.a = max(result.a, cover);
        cover = 0.0;
#else
        result.rgb *= 1.0 - cover;
#endif
        result.a = max(result.a, cover);
        hit.sign = sign;
        hit.p = p;
        hit.aa = aa;
        hit.weight = cover * shine;
    }
    // In the street the reflection is laid over the dark wet asphalt.
    result.rgb = lerp(float3(0.020, 0.017, 0.030), result.rgb, shine);
    return result;
}

// ---------------------------------------------------------------------------------------------
// Utility poles and power lines, close by in the street, in front of everything.

// A point `h` above the street, `d` away round the horizon at `phi`, as the eye sees it.
float3 Spot(float3 at)
{
    return float3(cos(at.x) * at.z, at.y - EYE, sin(at.x) * at.z);
}

// How much of a power line covers direction `n`: a cable from `from` to `to` (each round the
// horizon, height above the street, distance) sagging `sag` in the middle. The sag is straight
// down, so the point of the cable in the pixel's direction round the horizon is the point of the
// straight cable there, found in plan; then it is compared with the pixel up and down, measured
// across the cable as it runs on screen.
float Cable(float3 n, float3 from, float3 to, float sag, float pixel)
{
    float3 a = Spot(from);
    float3 b = Spot(to);
    float2 d = normalize(n.xz);
    float2 ab = b.xz - a.xz;
    float t = ((d.y * a.x) - (d.x * a.z)) / ((d.x * ab.y) - (d.y * ab.x));
    float tc = saturate(t);
    float3 spot = lerp(a, b, tc) - float3(0.0, sag * 4.0 * tc * (1.0 - tc), 0.0);
    float t2 = saturate(tc + 0.01);
    float3 next = lerp(a, b, t2) - float3(0.0, sag * 4.0 * t2 * (1.0 - t2), 0.0);
    float rise = atan2(spot.y, length(spot.xz));
    float2 run = float2(atan2((spot.x * next.z) - (spot.z * next.x), dot(spot.xz, next.xz)) * cos(rise), atan2(next.y, length(next.xz)) - rise);
    float across = abs(normalize(run + float2(1e-6, 0.0)).x);
    float off = abs(atan2(n.y, length(n.xz)) - rise) * across;
    float radius = 0.0009 / length(spot);
    float w = max(radius, 0.5 * pixel);
    float inside = step(0.0, t) * step(t, 1.0) * step(0.0, dot(d, spot.xz));
    return saturate(((w - off) / pixel) + 0.5) * saturate((radius / w) + 0.35) * inside;
}

// How much of a utility pole covers direction (phi, e): a tapering concrete pole `pole.z` away
// at `pole.x` round the horizon, `pole.y` tall, with two cross-arms and a transformer drum.
float Pole(float phi, float e, float pixel, float3 pole)
{
    float turn = phi - pole.x;
    turn -= TAU * round(turn / TAU);
    float ce = max(cos(e), 0.05);
    float2 p = float2(turn * pole.z, (pole.z * tan(clamp(e, -1.3, 1.45))) + EYE);
    float2 aa = float2(pixel * pole.z / ce, pixel * pole.z / (ce * ce));
    float top = pole.y;
    float r = lerp(0.0080, 0.0055, saturate(p.y / top));
    float cover = BoxCover(p, float4(-r, r, 0.0, top), aa);
    cover = max(cover, BoxCover(p, float4(-0.045, 0.045, top - 0.035, top - 0.029), aa));
    cover = max(cover, BoxCover(p, float4(-0.032, 0.032, top - 0.078, top - 0.072), aa));
    cover = max(cover, BoxCover(p, float4(0.008, 0.026, top - 0.17, top - 0.115), aa));
    // Insulators standing on the arms.
    float knob = BoxCover(float2(frac((p.x + 0.045) / 0.03) * 0.03, p.y), float4(0.012, 0.018, top - 0.029, top - 0.018), aa) * step(abs(p.x), 0.046);
    return max(cover, knob);
}

// Three poles round the street (where, height, distance) and the cables strung between them
// and to the buildings, some passing right overhead, with lower telephone lines below.
static const float3 POLES[3] = { float3(0.85, 0.62, 0.42), float3(2.45, 0.60, 0.45), float3(-1.90, 0.64, 0.40) };
static const float4 CABLE_FROM[16] = {
    float4(0.755, 0.59, 0.42, 0.06), float4(0.945, 0.59, 0.42, 0.06), float4(0.85, 0.546, 0.42, 0.05),
    float4(-1.80, 0.61, 0.40, 0.05), float4(-2.00, 0.61, 0.40, 0.06), float4(-1.80, 0.61, 0.40, 0.05), float4(-2.00, 0.61, 0.40, 0.06),
    float4(0.945, 0.59, 0.42, 0.03), float4(0.85, 0.546, 0.42, 0.02),
    float4(2.54, 0.57, 0.45, 0.03), float4(2.45, 0.528, 0.45, 0.02),
    float4(-1.80, 0.61, 0.40, 0.03), float4(-2.00, 0.61, 0.40, 0.03),
    float4(0.85, 0.44, 0.42, 0.04), float4(-1.90, 0.46, 0.40, 0.05), float4(2.45, 0.42, 0.45, 0.03) };
static const float3 CABLE_TO[16] = {
    float3(2.36, 0.57, 0.45), float3(2.54, 0.57, 0.45), float3(2.45, 0.528, 0.45),
    float3(0.755, 0.59, 0.42), float3(0.945, 0.59, 0.42), float3(2.54, 0.57, 0.45), float3(2.36, 0.57, 0.45),
    float3(-0.35, 0.44, 0.7), float3(-0.20, 0.38, 0.7),
    float3(3.40, 0.42, 0.7), float3(3.55, 0.36, 0.7),
    float3(-1.10, 0.48, 0.7), float3(-2.75, 0.40, 0.7),
    float3(2.45, 0.42, 0.45), float3(0.85, 0.44, 0.42), float3(3.90, 0.30, 0.7) };

// How much of the poles and power lines cover direction `n`.
float Wires(float3 n, float phi, float e, float pixel)
{
    float cover = 0.0;
    [loop]
    for (int k = 0; k < 3; k++)
    {
        cover = max(cover, Pole(phi, e, pixel, POLES[k]));
    }
    [loop]
    for (int c = 0; c < 16; c++)
    {
        cover = max(cover, Cable(n, CABLE_FROM[c].xyz, CABLE_TO[c], CABLE_FROM[c].w, pixel));
    }
    return cover;
}
