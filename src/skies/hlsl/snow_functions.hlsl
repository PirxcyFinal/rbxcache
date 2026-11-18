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

// The pace snow keeps off the sky clock: the rain's, which holds its speed over the crest of
// a swell, with a gentler log. The log keeps flakes changing while the clock sits near zero.
// The rain's races as the clock touches zero, and slow flakes show that as a blink where
// streaks of rain do not, so here it is half the size and levels off instead of racing.
float Pace(float clock)
{
    float c = min(saturate(clock), 0.999);
    return (0.65 * asin(c)) + (0.175 * log((1.0 + c) / (1.0 - c))) + (0.1 * log(c + 0.06));
}

// One sheet of snow, built the way the storm builds its rain. The sky clock runs back and
// forth and the shader cannot tell which way, so nothing here travels: a flake that slid down
// the screen would slide back up each time the clock turned. A flake shows at one spot, eases
// in and straight back out of opacity, and the next shows somewhere else. It is drawn as a
// soft round flake that its fall has smeared a little way upward, so it is plainly a flake
// and still reads as coming down whichever way the clock runs.
//   u       the screen in units of its height (x already scaled by the aspect ratio)
//   pixel   one screen pixel in the same units
//   cols    cells across one screen height
//   rows    cells down one screen height
//   rate    showings per run of the clock
//   size    a flake's half width as a share of the screen's height
float SnowSheet(float2 u, float pixel, float clock, float sheet, float cols, float rows, float rate, float size)
{
    float2 q = float2(u.x + (u.y * SLANT), u.y);
    float px = q.x * cols;
    float py = q.y * rows;
    float3 id = float3(floor(px), floor(py), sheet * 91.0);
    float lane = frac(px);
    float along = frac(py);
    // Neighbouring patches run at quite different speeds, so the whole sky never turns together.
    float speed = 0.5 + Hash3(id);
    float phase = Hash3(id + 0.37);
    float drive = rate * Pace(clock);
    float total = 0.0;
    for (int turn = 0; turn < 4; turn++)
    {
        // Four generations a quarter beat apart. A flake spends half its life easing in and
        // the other half easing out, with no hold between, so nothing appears or goes
        // quickly. One generation eases out exactly as another eases in, so the four always
        // add up to the same amount and the snow never pulses.
        float beat = (drive * speed) + phase + (turn * 0.25);
        float age = frac(beat);
        float fade = smoothstep(0.0, 0.5, age) * (1.0 - smoothstep(0.5, 1.0, age));
        float3 showing = id + float3(floor(beat) * 13.7, turn * 3.1, 1.0);
        // Flakes differ in size, and none is finer than the screen can show.
        float width = max(size * (0.7 + (0.6 * Hash3(showing + 6.6))), pixel * 1.1);
        float2 w = width * float2(cols, rows);
        // How far the fall draws the flake out upward, in half widths. Some stay nearly round.
        float smear = 1.2 + (0.8 * Hash3(showing + 4.4));
        // The whole flake, with the room it floats in, stays inside its cell, so none is cut off at an edge.
        float side = min(w.x * 2.7, 0.45);
        float top = min(w.y * smear * 1.8, 0.60);
        float bottom = min(w.y * 1.8, 0.30);
        float centre = side + ((1.0 - (2.0 * side)) * Hash3(showing + 2.2));
        float head = top + ((1.0 - top - bottom) * Hash3(showing + 5.1));
        // It floats a little to one side as it comes and goes, each flake its own way. A float
        // that runs back when the clock turns still looks like a flake in the air.
        centre += w.x * 0.9 * sin((beat * 2.2) + (Hash3(showing + 1.9) * 6.2831853));
        float across = (lane - centre) / w.x;
        float down = (along - head) / w.y;
        // Negative is up the screen: the upper half is drawn out and fainter, the lower half stays round.
        float up = saturate(-down / smear);
        down = down < 0.0 ? down / smear : down;
        float d = (across * across) + (down * down);
        // An even, soft-edged body rather than a point of glow.
        float flake = exp(-d * sqrt(d)) * (1.0 - (0.45 * up));
        float bright = 0.6 + (0.4 * Hash3(showing + 7.7));
        total += step(Hash3(showing + 8.4), DENSITY) * fade * flake * bright;
    }
    // The four generations add up to two flakes' worth; scaled to the three a held fade gave.
    return total * 1.5;
}
