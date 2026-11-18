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
    float3 moon = MoonDirection();
    float angle = acos(clamp(dot(n, moon), -1.0, 1.0));
    float up = saturate(n.y);

    // A dark sky, deep red low down and toward the moon, almost black overhead.
    float toward = saturate(dot(normalize(n.xz + 1e-5), normalize(moon.xz + 1e-5)));
    float3 sky = lerp(float3(0.26, 0.05, 0.05), float3(0.09, 0.02, 0.03), smoothstep(0.0, 0.30, up));
    sky = lerp(sky, float3(0.022, 0.010, 0.022), smoothstep(0.25, 0.9, up));
    sky += float3(0.35, 0.05, 0.03) * 0.30 * pow(toward, 4.0) * exp(-up / 0.25);
#if !ENV
    // A few faint stars, dimmed near the moon.
    float3 grid = n * 140.0;
    float3 box = floor(grid);
    float3 centre = box + 0.5 + ((float3(Hash3(box), Hash3(box + 7.1), Hash3(box + 13.7)) - 0.5) * 0.5);
    float star = step(0.982, Hash3(box + 21.3)) * (1.0 - smoothstep(0.0, 0.15, length(grid - centre)));
    sky += star * 0.30 * smoothstep(0.25, 0.7, up) * saturate(angle / 0.6) * float3(1.0, 0.85, 0.80);
#endif
    // The moon: a huge red disc with darker seas and a brighter rim of light round it.
    float radius = 0.13 * SIZE;
    float3 axis_x = normalize(float3(-moon.z, 0.0, moon.x));
    float3 axis_y = cross(moon, axis_x);
    float2 m = float2(dot(n, axis_x), dot(n, axis_y)) / radius;
    float face = Fill(angle - radius, pixel) * step(0.0, dot(n, moon));
    float seas = smoothstep(0.45, 0.75, Noise2((m * 2.2) + 3.0)) * 0.35 + (smoothstep(0.55, 0.8, Noise2((m * 6.0) + 9.0)) * 0.15);
    float limb = sqrt(saturate(1.0 - dot(m, m)));
    float3 disc = lerp(float3(0.95, 0.30, 0.12), float3(0.55, 0.08, 0.05), seas) * (0.55 + (0.45 * limb));
    sky += float3(0.70, 0.10, 0.05) * 0.45 * exp(-max(angle - radius, 0.0) / 0.10);
    sky += float3(0.50, 0.05, 0.03) * 0.20 * exp(-max(angle - radius, 0.0) / 0.35);
    sky = lerp(sky, disc, face);
    // Thin ragged cloud drifting across, dark against the moon, lit red at its edges near it.
    float2 deck = ((n.xz / (abs(n.y) + 0.20)) * 1.2) + float2(0.08 * sin(sway * 0.4), 0.03 * cos(sway * 0.3));
    float cloud = smoothstep(0.50, 0.78, Cloud(deck)) * CLOUDS * smoothstep(0.02, 0.10, up);
    float3 cloud_colour = lerp(float3(0.05, 0.015, 0.02), float3(0.45, 0.08, 0.05), exp(-angle / 0.25) * (1.0 - smoothstep(0.62, 0.78, Cloud(deck))));
    sky = lerp(sky, cloud_colour, cloud * 0.85);

    // Below the horizon, dark ground.
    float3 ground = float3(0.018, 0.008, 0.012);
    sky = lerp(sky, ground, Fill(e, pixel));

    // Bare trees along the horizon, black against the red glow.
    [branch]
    if ((e > -0.02) && (e < 0.16))
    {
        float cells = 48.0;
        float at = (phi + PI) / (TAU / cells);
        float place = fmod(floor(at), cells);
        float x = (frac(at) - 0.5) * (TAU / cells) * cos(e);
        float h = 0.05 + (0.11 * Hash1(place + 3.1));
        float present = step(Hash1(place + 9.7), 0.62);
        float2 p = float2(x - (0.02 * (Hash1(place + 5.3) - 0.5)), e);
        float tree = Fill(Tree(p, h, place * 7.0), pixel) * present;
        // A low ridge of woodland the trees stand on.
        float ridge = 0.008 + (0.010 * Noise2(float2(phi * 6.0, 2.0))) + (0.004 * Noise2(float2(phi * 40.0, 5.0)));
        float land = Fill(e - ridge, pixel);
        sky = lerp(sky, float3(0.012, 0.004, 0.008), max(tree, land));
    }

    // Fog lying low over the ground, glowing faintly red, thinning as it rises.
    float fog = exp(-max(e + 0.01, 0.0) / 0.035) * FOG;
    fog *= 0.75 + (0.25 * Noise2(float2((phi * 8.0) + (0.3 * sin(sway * 0.5)), e * 30.0)));
    sky = lerp(sky, float3(0.16, 0.04, 0.04) + (float3(0.22, 0.04, 0.02) * pow(toward, 3.0)), saturate(fog) * 0.45);

#if !ENV
    // Bats.
    float bat = max(Bats(phi, e, pixel, clock, 0.10, 0.06, 1.0), Bats(phi + 1.7, e, pixel, clock, 0.06, 0.05, 2.0));
    bat *= smoothstep(0.03, 0.08, e) * (1.0 - smoothstep(0.55, 0.8, e));
    sky = lerp(sky, float3(0.01, 0.004, 0.006), bat);
#endif
    return sky * LEVEL;
