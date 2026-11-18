#if ENV
    float clock = 0.5;   // reflections get a still picture
#else
    float clock = saturate(TIME);
#endif
    float phi = atan2(n.z, n.x);            // round the horizon, in radians
    float e = asin(clamp(n.y, -1.0, 1.0));  // elevation, in radians
    // How far one screen pixel turns the view, in radians: the width of a smooth edge.
    float pixel = max(length(fwidth(n)), 0.0002);
    float3 sun = SunDirection();
    float sway = Ease(clock) * TAU;
    float3 sky;
    [branch]
    if (e >= 0.0)
    {
        sky = SkyColour(n, sun);
        // The sun's disc, a little deeper in colour toward its rim.
        float angle = acos(clamp(dot(n, sun), -1.0, 1.0));
        float disc = Fill(angle - 0.022, pixel);
        sky = lerp(sky, float3(1.0, 0.90, 0.68) * (1.15 + (0.25 * saturate(1.0 - (angle / 0.022)))), disc * SUN);
        // A deck of soft cloud, lit from beneath by the low sun: gold and pink toward it, lilac
        // away from it, darker on top. It drifts a little to and fro with the clock.
        float2 deck = ((n.xz / (n.y + 0.12)) * 0.8) + float2(0.06 * sin(sway * 0.5), 0.0);
        float density = smoothstep(0.45, 0.80, Cloud(deck)) * CLOUDS;
        float toward = saturate(dot(normalize(n.xz + 1e-5), normalize(sun.xz + 1e-5)));
        float3 lit = lerp(float3(0.55, 0.32, 0.50), float3(1.00, 0.55, 0.32), pow(toward, 2.0));
        lit = lerp(lit, float3(1.00, 0.78, 0.45), exp(-angle / 0.25) * 0.8);
        float3 cloud = lerp(lit, (lit * 0.45) + float3(0.05, 0.04, 0.10), smoothstep(0.4, 0.8, Cloud((deck * 2.1) + 7.0)));
        sky = lerp(sky, cloud, saturate(density) * smoothstep(0.0, 0.05, e) * 0.9);
#if !ENV
        // Gulls flapping, low over the sea.
        float gull = max(Gulls(phi, e, pixel, clock, 0.09, 0.030), Gulls(phi + 1.3, e, pixel, clock, 0.05, 0.022));
        gull *= smoothstep(0.02, 0.06, e) * (1.0 - smoothstep(0.30, 0.50, e));
        sky = lerp(sky, float3(0.05, 0.04, 0.06), gull);
#endif
    }
    else
    {
        // The sea, seen from just above it. Its surface is tilted wave by wave with noise that
        // moves with the clock, and each tilt sends back a different part of the sky, so a path
        // of light runs out toward the sun, broken into glints that sparkle as the waves move.
        float2 sea = n.xz / max(-n.y, 0.004);
        float2 wave_at = sea * 0.35;
        float2 tilt = float2(Noise2(wave_at + float2(sway * 0.3, 0.0)), Noise2(wave_at + 17.0 - float2(0.0, sway * 0.3))) - 0.5;
        tilt += (float2(Noise2((wave_at * 4.0) + 3.0 + sway), Noise2((wave_at * 4.0) + 11.0 - sway)) - 0.5) * 0.5;
        float flatten = saturate(-e * 6.0);
        float3 normal = normalize(float3(tilt.x * 0.25 * flatten, 1.0, tilt.y * 0.25 * flatten));
        float3 r = reflect(n, normal);
        r.y = abs(r.y);
        float fresnel = 0.04 + (0.96 * pow(1.0 - saturate(-n.y), 5.0));
        sky = lerp(float3(0.02, 0.05, 0.10), SkyColour(r, sun), saturate(fresnel * 1.4));
#if !ENV
        // Glints: tiny bright points where a wave faces the sun just right, twinkling.
        float glint = pow(saturate(dot(r, sun)), 900.0) * (0.4 + (0.6 * Noise2((sea * 3.0) + (sway * 2.0))));
        sky += float3(1.0, 0.85, 0.60) * glint * 2.5 * SUN;
#endif
        // A haze along the horizon where the sea meets the sky.
        sky = lerp(sky, SkyColour(float3(n.x, 0.0, n.z), sun), exp(e / 0.012));
    }
    return sky * LEVEL;
