#if ENV
    float clock = 0.5;   // reflections get a still picture
#else
    float clock = saturate(TIME);
#endif
    float phi = atan2(n.z, n.x);            // round the horizon, in radians
    float e = asin(clamp(n.y, -1.0, 1.0));  // elevation, in radians
    // How far one screen pixel turns the view, in radians: the width of a smooth edge.
    float pixel = max(length(fwidth(n)), 0.0002);
    float sway = Ease(clock) * TAU;
    float bearing = SunBearing();
    float turn = phi - bearing;
    turn -= TAU * round(turn / TAU);
    float3 sky;
    [branch]
    if (e >= 0.0)
    {
        // Deep purple overhead, through magenta to a hot pink glow along the horizon.
        float up = saturate(n.y);
        sky = lerp(float3(0.85, 0.20, 0.55), float3(0.36, 0.08, 0.42), smoothstep(0.0, 0.16, up));
        sky = lerp(sky, float3(0.05, 0.02, 0.14), smoothstep(0.12, 0.75, up));
        sky += float3(1.0, 0.35, 0.45) * 0.35 * exp(-up / 0.05) * exp(-(turn * turn) / 0.5);
#if !ENV
        // Stars, twinkling slowly.
        float3 grid = n * 150.0;
        float3 box = floor(grid);
        float3 centre = box + 0.5 + ((float3(Hash3(box), Hash3(box + 7.1), Hash3(box + 13.7)) - 0.5) * 0.5);
        float twinkle = 0.6 + (0.4 * cos((Hash3(box + 3.3) * TAU) + sway));
        float star = step(0.975, Hash3(box + 21.3)) * (1.0 - smoothstep(0.0, 0.15, length(grid - centre))) * twinkle;
        sky += star * 0.55 * smoothstep(0.15, 0.6, up) * float3(0.9, 0.85, 1.0);
#endif
        // The sun: a big disc, yellow at its top to pink at its foot, cut by bands that grow
        // wider toward the bottom and drift slowly with the clock.
        float2 s = float2(turn * cos(e), e - 0.11);
        float radius = 0.22 * SIZE;
        float r = length(s);
        float across = saturate(0.5 - (s.y / (2.0 * radius)));
        float3 disc = lerp(float3(1.00, 0.88, 0.25), float3(1.00, 0.22, 0.55), across);
        float band = frac(((s.y / radius) * 5.0) + (0.25 * sin(sway * 0.5)));
        float gap = (s.y < radius * 0.1) ? step(band, lerp(0.05, 0.45, across)) : 0.0;
        float cover = Fill(r - radius, pixel) * (1.0 - gap);
        sky += float3(1.0, 0.35, 0.50) * 0.25 * exp(-max(r - radius, 0.0) / 0.10);
        sky = lerp(sky, disc, cover);
        // Wireframe mountains in front of it: dark, lines of light following their shape, a
        // bright edge along the ridge.
        float top = Ridge(phi, bearing);
        float inside = Fill(e - top, pixel);
        float3 mountain = float3(0.06, 0.02, 0.10);
        // How far the point is from the nearest contour line and the nearest upright rib, in
        // radians, so the lines are drawn about a pixel wide.
        float contour = abs(frac((e / max(top, 0.002)) * 7.0) - 0.5) * (max(top, 0.002) / 7.0);
        float wire = saturate(1.0 - (contour / (pixel * 0.8)));
        float rib = abs(frac((phi + PI) * 90.0 / TAU) - 0.5) * (TAU / 90.0) * cos(e);
        wire = max(wire, saturate(1.0 - (rib / (pixel * 0.8))));
        mountain += float3(0.20, 0.55, 1.00) * 0.45 * wire * (0.4 + (0.6 * (e / max(top, 0.002))));
        float edge = exp(-abs(e - top) / (pixel * 1.5));
        sky = lerp(sky, mountain, inside);
        sky += float3(0.35, 0.80, 1.00) * 0.7 * edge;
    }
    else
    {
        // The grid floor, running out to the horizon. Its lines are filtered so they fade into a
        // glow far off rather than shimmering; pulses of light run along them.
        // Turned so one set of lines runs straight out toward the sun.
        float2 flat_ = n.xz / max(-n.y, 0.002);
        float2 floor_at = float2((flat_.x * cos(bearing)) + (flat_.y * sin(bearing)), (-flat_.x * sin(bearing)) + (flat_.y * cos(bearing))) * 0.9 * GRID;
        float2 width = max(fwidth(floor_at), 0.0001);
        float2 g = abs(frac(floor_at - 0.5) - 0.5) / width;
        float line_ = 1.0 - saturate(min(g.x, g.y) - 0.5);
        float far_ = saturate((max(width.x, width.y) - 0.15) / 0.35);
        line_ = lerp(line_, 0.18, far_);
        // Bands of light a few lines apart, so many show at once and the floor as a whole stays
        // as bright.
        float pulse = 0.75 + (0.25 * sin((length(floor_at) * 0.8) - (sway * 1.5)));
        float3 neon = lerp(float3(1.00, 0.20, 0.70), float3(0.20, 0.70, 1.00), saturate(-n.y * 3.0));
        sky = float3(0.03, 0.01, 0.06) + (neon * line_ * pulse * 1.25);
        // The sun's light along the floor, and the horizon's pink haze.
        sky += float3(1.0, 0.30, 0.55) * 0.35 * exp(e / 0.06) * exp(-(turn * turn) / 0.3);
        sky = lerp(sky, float3(0.85, 0.20, 0.55), exp(e / 0.008) * 0.8);
    }
    return sky * LEVEL;
