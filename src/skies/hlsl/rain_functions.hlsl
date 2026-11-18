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

// Cloud as layers of noise, 0 to 1.
float Cloud(float2 p)
{
    float sum = 0.0;
    float weight = 0.5;
    for (int octave = 0; octave < 5; octave++)
    {
        sum += weight * Noise2(p);
        p = (p * 2.03) + 19.19;
        weight *= 0.5;
    }
    return sum;
}

// A short pulse: 1 when `value` is at `centre`, 0 once it is `width` away.
float Pulse(float value, float centre, float width)
{
    float closeness = saturate(1.0 - (abs(value - centre) / width));
    return closeness * closeness;
}

// SOFT picks how the rain is drawn and timed. 0 is the storm's: crisp streaks that hold and
// then crossfade, on a pace that never lets up. 1 is the calm rain's: streaks blurred by their
// fall that are always easing in or out, on a pace that does not race. Only 0 and 1 keep the
// amount of rain steady; in between it would pulse.

// The pace rain keeps off the sky clock. The clock swells and shrinks, and timed straight off
// it a streak sits still for seconds at the top of a swell and again while the clock sits near
// zero. The arcsine keeps the pace steadier than the clock. A share of atanh is added because
// the biggest swells stall as the clock nears 1, and atanh changes much faster there, so the
// rain holds its speed over the crest. The log keeps it moving while the clock sits near zero.
// It races as the clock touches zero, which crisp streaks hide and slow soft ones show as a
// blink, so the soft rain's log is half the size and levels off instead.
float Pace(float clock)
{
    float c = min(saturate(clock), 0.999);
    return (0.65 * asin(c)) + (0.175 * log((1.0 + c) / (1.0 - c))) + (lerp(0.2, 0.1, SOFT) * log(max(c, 0.0001) + (0.06 * SOFT)));
}

// One sheet of rain. The sky clock runs back and forth and the shader cannot tell which way,
// so streaks do not travel: each flashes in at one spot and fades, and the next appears
// somewhere else. A streak is faint at the top and brightest at its lower end, so it reads as
// a falling drop whichever way the clock runs. Streaks sit apart in wide cells.
//   u       the screen in units of its height (x already scaled by the aspect ratio)
//   cols    cells across one screen height; fewer means wider gaps between streaks
//   rows    cells down one screen height
//   rate    showings per run of the clock
//   streak  a streak's length as a share of its cell
//   thin    a streak's half width in screen pixels
//   chance  share of showings that hold a streak
float RainSheet(float2 u, float clock, float sheet, float cols, float rows, float slant, float rate, float streak, float thin, float chance)
{
    float2 q = float2(u.x + (u.y * slant), u.y);
    float px = q.x * cols;
    float column = floor(px);
    float lane = frac(px);
    float py = q.y * rows;
    float3 id = float3(column, floor(py), sheet * 91.0);
    float along = frac(py);
    // How much of a lane one screen pixel covers, so a streak's width is set in pixels.
    float pixel = max(abs(ddx(px)), 0.00001);
    // Neighbouring patches run at quite different speeds, so the whole sky never turns together.
    float speed = 0.5 + (1.0 * Hash3(id));
    float phase = Hash3(id + 0.37);
    float drive = rate * Pace(clock);
    // The share of its life a streak spends easing in, and again easing out.
    float edge = 0.25 + (0.25 * SOFT);
    float total = 0.0;
    for (int turn = 0; turn < 4; turn++)
    {
        // Four generations a quarter beat apart. A crisp streak is fully there for the middle
        // half of its life and only crossfades at the ends, so while the sky clock turns
        // around and runs back, a drop in the middle of its life does not change. A soft one
        // eases in for half its life and out for the other half. Either way one generation
        // eases out exactly as another eases in, so the four always add up to the same amount
        // and the rain never pulses.
        float beat = (drive * speed) + phase + (turn * 0.25);
        float age = frac(beat);
        float fade = smoothstep(0.0, edge, age) * (1.0 - smoothstep(1.0 - edge, 1.0, age));
        float3 showing = id + float3(floor(beat) * 13.7, turn * 3.1, 1.0);
        // A streak sits inside its cell, so nothing is cut off at a cell edge.
        float span = streak * (1.0 + (0.6 * SOFT)) * (0.55 + (0.45 * Hash3(showing + 6.6)));
        float centre = 0.15 + (0.70 * Hash3(showing + 2.2));
        float head = 0.03 + (Hash3(showing + 5.1) * (0.94 - span));
        float v = (along - head) / span;
        float gap = (lane - centre) / (pixel * thin);
        // Crisp: an even core a pixel or two wide that fades in at the top and ends quickly at
        // the bottom, brightest low down.
        float body = smoothstep(0.0, 0.35, v) * (1.0 - smoothstep(0.85, 1.0, v)) * (0.45 + (0.55 * saturate(v)));
        float across = exp(-gap * gap);
        // Soft: longer, easing in and out along its whole length, with a wider core and a faint
        // spread to either side, and a little dimmer so the wider streak adds no more rain.
        float smear = smoothstep(0.0, 0.60, v) * (1.0 - smoothstep(0.55, 1.0, v)) * (0.70 + (0.30 * saturate(v)));
        float spread = (0.75 * exp(-gap * gap * 0.60)) + (0.25 * exp(-gap * gap * 0.10));
        float bright = 0.6 + (0.4 * Hash3(showing + 7.7));
        total += step(Hash3(showing + 8.4), chance) * fade * lerp(body * across, smear * spread * 0.7, SOFT) * bright;
    }
    // The four generations add up to 4 * (1 - edge) streaks; this keeps the amount crisp rain gives.
    return total * 0.45 * (0.75 / (1.0 - edge));
}

// Rain over the whole view, 0 to 1. Three sheets of fine streaks a pixel or two wide, leaning
// with the wind: nearer ones are longer, wider and change more slowly, far ones are shorter,
// finer and change a little faster. Rain and Storm both draw these sheets; SOFT is what differs.
//   screen  the pixel's place on screen, 0 to 1 across and down
//   aspect  the screen's width over its height
float Downpour(float2 screen, float aspect, float clock)
{
    float2 u = float2(screen.x * aspect, screen.y);
    float rain = 0.0;
    rain += RainSheet(u, clock, 0.0, 18.0, 5.0, SLANT, SPEED * 3.2, 0.34, 1.4, 0.50) * 1.20;
    rain += RainSheet(u, clock, 1.0, 28.0, 7.0, SLANT, SPEED * 4.8, 0.30, 1.1, 0.45) * 0.90;
    rain += RainSheet(u, clock, 2.0, 40.0, 9.0, SLANT, SPEED * 7.2, 0.26, 0.9, 0.40) * 0.65;
    // Only the very edge eases off, so there is no band where the rain stops.
    rain *= smoothstep(0.0, 0.05, screen.y) * (1.0 - smoothstep(0.92, 1.0, screen.y));
    return 1.0 - exp(-rain * RAIN);
}
