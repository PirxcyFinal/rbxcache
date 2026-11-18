#if ENV
    float clock = 0.5;   // reflections get a still picture
#else
    float clock = saturate(TIME);
#endif
    // Slow sway, and the only thing that moves the curtains. A cosine of the clock is level
    // where the clock touches zero and turns around, so the curtains ease through every turn,
    // and at this size the finest rays slide well under once a second.
    float3 swing = cos(float3(1.0, 2.0, 3.0) * 3.14159265 * clock) * SPEED;

    float u = atan2(n.z, n.x) / TAU;                       // once round the horizon
    float e = asin(saturate(n.y)) / radians(TOP);          // 0 on the horizon, 1 at the top of the display
    float3 light = float3(0.0, 0.0, 0.0);
    for (int k = 0; k < 3; k++)
    {
        float4 shape = SHAPE[k];
        float4 a = PHASE_A[k];
        float4 b = PHASE_B[k];
        float way = (k == 1) ? -1.0 : 1.0;
        // The foot of the curtain. Whole numbers of waves round the horizon, so there is no seam.
        float foot = shape.x + (shape.y * (sin((TAU * u) + a.x + (way * 0.30 * swing.x))
            + (0.5 * sin((TAU * 3.0 * u) + a.y - (way * 0.70 * swing.y)))
            + (0.25 * sin((TAU * 7.0 * u) + a.z + (way * 1.0 * swing.z)))));
        float above = e - foot;
        float body = exp(-max(above, 0.0) / shape.z) * saturate((above + 0.004) / 0.008);
        // Rays: upright stripes, coarse to fine, that shift along the curtain.
        float rays = (0.30 * sin((TAU * 9.0 * u) + a.w + (way * 0.45 * swing.y)))
            + (0.22 * sin((TAU * 14.0 * u) + b.x - (way * 0.65 * swing.x)))
            + (0.20 * sin((TAU * 22.0 * u) + b.y + (way * 0.85 * swing.z)))
            + (0.14 * sin((TAU * 37.0 * u) + b.z - (way * 1.00 * swing.y)))
            + (0.14 * sin((TAU * 61.0 * u) + b.w + (way * 1.30 * swing.x)));
        float bright = 0.45 + (0.55 * (0.5 + (0.5 * sin((TAU * 2.0 * u) + (b.x * 1.7) + (way * 0.45 * swing.x)))));
        float rise = saturate(above / (shape.z * 2.2));
        light += body * (0.55 + (0.45 * rays)) * bright * shape.w * lerp(float3(0.12, 1.00, 0.45), float3(0.55, 0.22, 0.95), rise);
    }
    light *= saturate((1.0 - e) / 0.12) * step(0.0, n.y);
    // Nothing brightens or dims the whole aurora at once. A ripple that did, timed off the
    // clock, ran at tens of cycles a second and showed as a flicker.
    light = 1.0 - exp(-light * 1.4);

    float3 night = lerp(float3(0.010, 0.018, 0.045), float3(0.030, 0.050, 0.100), pow(1.0 - saturate(abs(n.y)), 3.0));
    float3 sky = (rgb * DIM) + (night * (1.0 - DIM));

#if !ENV
    // Stars: the directions are cut into small boxes, and a few boxes hold a star.
    float3 grid = n * 160.0;
    float3 box = floor(grid);
    float3 centre = box + 0.5 + ((float3(Hash3(box), Hash3(box + 7.1), Hash3(box + 13.7)) - 0.5) * 0.6);
    float size = Hash3(box + 3.3);
    float star = step(0.95, Hash3(box + 21.3)) * (1.0 - smoothstep(0.0, 0.14 + (0.12 * size), length(grid - centre)));
    // A slow twinkle, each star at its own point in it. No log of the clock: that races as the
    // clock touches zero and made the stars flash.
    float twinkle = 0.80 + (0.20 * cos((size * TAU) + (1.6 * cos(TAU * clock))));
    float dimmed = 1.0 - (0.7 * max(light.x, max(light.y, light.z)));
    sky += star * (0.35 + (0.65 * size * size)) * twinkle * dimmed * saturate(n.y * 8.0) * STARS * float3(0.90, 0.95, 1.0);
#endif
    return sky + (light * GLOW);
