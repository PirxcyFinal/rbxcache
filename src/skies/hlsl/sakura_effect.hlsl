#if ENV
    float clock = 0.5;   // reflections get a still picture
#else
    float clock = saturate(TIME);
#endif
    // The wind, back and forth as the clock is. It runs off the eased glide, which comes to a
    // stop where the clock touches zero, so nothing in the wind jerks when the clock turns there.
    float breeze = Glide(clock);
    float sway = sin(TAU * breeze);

    float u = atan2(n.z, n.x) / TAU;    // once round the horizon
    float e = asin(clamp(n.y, -1.0, 1.0));  // elevation, in radians
    // How far one screen pixel turns the view, in radians: the width of a smooth edge.
    float footprint = max(length(fwidth(n)), 0.0002);

    // A pale spring sky, rosy toward the horizon, nearly white where it shows through the blossom.
    float up = saturate(n.y);
    float3 sky = lerp(float3(0.97, 0.86, 0.90), float3(0.80, 0.87, 0.97), smoothstep(0.0, 0.35, up));
    sky = lerp(sky, float3(0.88, 0.92, 0.98), smoothstep(0.30, 1.0, up));
    // A few soft fair-weather clouds.
    float2 deck = (n.xz / (abs(n.y) + 0.30)) * 0.9;
    float puff = smoothstep(0.52, 0.75, Cloud(deck + float2(0.02 * sway, 0.0)));
    sky = lerp(sky, float3(1.0, 0.97, 0.98), puff * 0.75 * smoothstep(0.02, 0.15, up));
    float3 haze = float3(0.96, 0.86, 0.90);

    // Far hills, lavender in the haze.
    float ridge = 0.018 + (0.030 * Noise2(float2(u * 14.0, 3.7))) + (0.012 * Noise2(float2(u * 41.0, 9.1)));
    float hills = 1.0 - smoothstep(ridge - 0.002, ridge + 0.002, e);
    sky = lerp(sky, lerp(float3(0.70, 0.70, 0.84), haze, 0.35), hills);

    // The ground, under a scatter of fallen petals, lost in the haze toward the horizon.
    float below = saturate(-n.y * 6.0);
    float2 lawn = (n.xz / (abs(n.y) + 0.05)) * 3.0;
    float fallen = smoothstep(0.50, 0.85, Cloud(lawn * 3.0));
    float3 ground = lerp(float3(0.42, 0.56, 0.35), float3(0.93, 0.76, 0.83), fallen * 0.7);
    sky = lerp(sky, lerp(haze, ground, smoothstep(0.0, 0.45, -n.y) * 0.85), below);

    // Two rows of cherry trees round the horizon: a far row, small and hazy, and a near row.
    // Above the tallest of them, or below their feet, they are not worked out at all.
    [branch]
    if (e > -0.11 && e < 0.36 * TREES)
    {
        float4 distant = Grove(u, e, 26.0, -0.012, 0.050 * TREES, 1.0, sway, footprint);
        sky = lerp(sky, lerp(distant.rgb, haze, 0.45), distant.a);
        float4 close = Grove(u + 0.013, e, 13.0, -0.035, 0.105 * TREES, 2.0, sway, footprint);
        sky = lerp(sky, lerp(close.rgb, haze, 0.12), close.a);
    }

    // Overhead: looking up into the cherry tree. The roof coordinates and their size on screen
    // are worked out for every pixel, outside any branch, so the smooth edges have what they need.
    // The branches hold still; only the blossom moves in the wind.
    float2 c = n.xz / max(n.y, 0.02);
    float px = max(length(fwidth(c)), 0.0001);
    [branch]
    if (n.y > 0.15)
    {
        float2 tree = Canopy(c);
        float canopy = smoothstep(0.20, 0.45, n.y);
        // The blossom rustles: each patch of it sways a little, at its own point in the sway,
        // with a slow wave across the tree, so the breeze seems to pass through in gusts rather
        // than move the whole canopy as one sheet.
        float gust = (Noise2((c * 1.1) + 5.0) * TAU * 1.5) + dot(c, float2(1.2, 0.7));
        float2 b = c + (0.011 * float2(sin((TAU * breeze * 1.4) + gust), 0.6 * sin((TAU * breeze * 1.1) + (gust * 1.7) + 1.3)));
        // The flowers crowd along the twigs, in clumps, with the sky showing between.
        float lump = Cloud((b * 2.2) + 3.1);
        float density = (1.0 - smoothstep(0.12, 0.85, tree.y + ((lump - 0.5) * 0.60))) * canopy;
        // Blossom behind, deep pink and soft, deepest where it is thickest.
        float mass = smoothstep(0.20, 0.50, density * (0.60 + (0.9 * Cloud((b * 4.0) + 7.7))));
        float3 behind = lerp(float3(0.80, 0.42, 0.58), float3(0.98, 0.74, 0.85), Cloud((b * 7.0) + 1.3));
        sky = lerp(sky, behind, mass * 0.92);
        // The branches, nearly black against the light, fading out where the blossom does.
        float bark = (1.0 - smoothstep(-px, px, tree.x)) * smoothstep(0.18, 0.42, n.y);
        sky = lerp(sky, float3(0.11, 0.07, 0.09), bark);
        // Flowers in front, larger and smaller, mostly off the bark of the limbs.
        density *= lerp(0.25, 1.0, smoothstep(-0.02, 0.05, tree.x));
        float4 big = Flowers(b, 14.0, 0.0, px, density);
        sky = lerp(sky, big.rgb, big.a);
        float4 small = Flowers(b + 0.37, 22.0, 1.0, px, density * 0.9);
        sky = lerp(sky, small.rgb, small.a);
    }
    // The trunk comes up past the horizon to the fork, in front of the trees out there. High up
    // it is drawn on the roof; low down, where the roof stretches toward the horizon, as an
    // upright band. Its edge is a pixel wide across the trunk, which on the roof is wider the
    // farther out it is.
    float trunk_high = Trunk(c) / (footprint * sqrt(1.0 + dot(c, c)));
    float trunk_low = TrunkLow(u) / footprint;
    float trunk = lerp(trunk_low, trunk_high, smoothstep(0.03, 0.08, n.y));
    sky = lerp(sky, float3(0.11, 0.07, 0.09), (1.0 - smoothstep(-1.0, 1.0, trunk)) * step(-0.95, n.y));

    // Down to the brightness of an ordinary sky. Pale blossom at full strength is close to
    // white, and a game's exposure and bloom turn that into a glare.
    // Dimmed pastels go grey, so the colour is lifted as the light comes down.
    float grey = dot(sky, float3(0.30, 0.59, 0.11));
    sky = saturate(lerp(float3(grey, grey, grey), sky, 1.45)) * LEVEL;

#if !ENV
    // Petals gliding down the sky, from near to far, fixed in the sky like the aurora's stars,
    // so turning the camera sweeps past them. Nearer ones glide farther. They are close and
    // catch the light, so they stay a little brighter than the sky behind them.
    float phi = u * TAU;
    float3 petal_light = float3(1.00, 0.96, 0.97) * min(LEVEL * 1.45, 1.0);
    float3 petal_deep = float3(0.98, 0.80, 0.87) * min(LEVEL * 1.45, 1.0);
    float2 p1 = PetalSky(phi, e, footprint, clock, 0.0, 0.20, 0.07 * GLIDE, SPEED * 0.9, 0.0135);
    float2 p2 = PetalSky(phi, e, footprint, clock, 1.0, 0.12, 0.05 * GLIDE, SPEED * 1.1, 0.0088);
    float2 p3 = PetalSky(phi, e, footprint, clock, 2.0, 0.076, 0.033 * GLIDE, SPEED * 1.3, 0.0056);
    sky = lerp(sky, lerp(petal_deep, petal_light, p3.x), p3.y * 0.75);
    sky = lerp(sky, lerp(petal_deep, petal_light, p2.x), p2.y * 0.88);
    sky = lerp(sky, lerp(petal_deep, petal_light, p1.x), p1.y);
#endif
    return sky;
