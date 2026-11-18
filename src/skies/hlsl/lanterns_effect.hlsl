#if ENV
    float clock = 0.5;   // reflections get a still picture
#else
    float clock = saturate(TIME);
#endif
    float phi = atan2(n.z, n.x);            // round the horizon, in radians
    float e = asin(clamp(n.y, -1.0, 1.0));  // elevation, in radians
    // How far one screen pixel turns the view, in radians: the width of a smooth edge.
    float pixel = max(length(fwidth(n)), 0.0002);

    // Above the horizon the scene itself. Below it the lake gives the scene back upside down,
    // broken by slow ripples, a little blurred, darker, and brightest near the far shore where
    // the water is seen at a glance.
    float below = step(e, 0.0);
    float ripple = below * 0.0015 * sin((phi * 60.0) + (Noise2(float2(phi * 12.0, e * 30.0)) * 4.0) + (Ease(clock) * TAU * 1.5)) * saturate(-e * 12.0);
    float3 seen = (below > 0.5) ? float3(n.x, -n.y, n.z) : n;
    float3 colour = Scene(phi + ripple, abs(e) + (ripple * 0.5), pixel * (1.0 + below), clock, seen);
    if (below > 0.5)
    {
        float shine = 0.22 + (0.48 * exp(e / 0.08));
        colour = lerp(float3(0.010, 0.012, 0.025), colour, shine);
    }
    return colour * LEVEL;
