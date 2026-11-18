// Synthwave Sunset: a big striped sun sinking behind wireframe mountains, a glowing grid running
// out to the horizon, stars over a purple sky. The grid cannot race toward the viewer on a clock
// that runs back and forth, so pulses of light run along its lines instead, which look the same
// either way.

static const float TAU = 6.2831853;
static const float PI = 3.14159265;

float Hash1(float x) { return frac(sin(x * 127.1) * 43758.5453); }
float Hash2(float2 p) { return frac(sin(dot(p, float2(127.1, 311.7))) * 43758.5453); }
float Hash3(float3 p) { return frac(sin(dot(p, float3(127.1, 311.7, 74.7))) * 43758.5453); }

// Smooth noise along a line that repeats every `period` (a whole number), so it closes up round
// the horizon without a seam.
float NoiseLoop(float x, float period)
{
    float i = floor(x);
    float f = frac(x);
    f = f * f * (3.0 - (2.0 * f));
    return lerp(Hash1(fmod(i, period)), Hash1(fmod(i + 1.0, period)), f);
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

// Where the sun sits: on the bearing of the game's own sun so the shadows lean the right way,
// or toward the south-west if the game gives no sun.
float SunBearing()
{
    float3 lamp = -G[16].xyz;
    return (dot(lamp.xz, lamp.xz) > 1e-6) ? atan2(lamp.z, lamp.x) : 0.6;
}

// The mountains' height at bearing `phi`: jagged peaks, highest either side of the sun and low
// in front of it so the sun shows over them, on whole numbers of waves round the horizon so
// there is no seam. In radians of elevation.
float Ridge(float phi, float sun)
{
    float turn = phi - sun;
    turn -= TAU * round(turn / TAU);
    float peaks = 0.0;
    float amp = 0.5;
    float period = 14.0;
    float around = (phi + PI) / TAU;
    for (int k = 0; k < 4; k++)
    {
        // Folded noise gives sharp ridges rather than round hills.
        peaks += amp * (1.0 - abs((2.0 * NoiseLoop((around * period) + (k * 31.0 * period), period)) - 1.0));
        amp *= 0.5;
        period *= 2.0;
    }
    float valley = 1.0 - (0.75 * exp(-(turn * turn) / 0.08));
    return (0.012 + (0.17 * pow(peaks, 1.6))) * valley;
}
