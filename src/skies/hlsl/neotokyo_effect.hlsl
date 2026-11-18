#if ENV
    float clock = 0.5;   // reflections get a still picture
#else
    float clock = saturate(TIME);
#endif
    float phi = atan2(n.z, n.x);            // round the horizon, in radians
    float u = phi / TAU;                    // once round the horizon
    float e = asin(clamp(n.y, -1.0, 1.0));  // elevation, in radians
    // How far one screen pixel turns the view, in radians: the width of a smooth edge.
    float pixel = max(length(fwidth(n)), 0.0002);
    float up = saturate(n.y);

    // Night over a city: deep indigo overhead, through violet to a magenta glow along the
    // horizon, where the city lights the haze.
    float3 haze = float3(0.27, 0.10, 0.25);
    float3 sky = lerp(haze, float3(0.10, 0.045, 0.16), smoothstep(0.0, 0.20, up));
    sky = lerp(sky, float3(0.014, 0.016, 0.048), smoothstep(0.14, 0.80, up));
    // Low rain cloud, lit from underneath by the city, so brightest toward the horizon.
    float2 deck = (n.xz / (abs(n.y) + 0.25)) * 1.1;
    float cloud = saturate((Cloud(deck) * 0.75) + (Cloud((deck * 2.2) + 31.0) * 0.25));
    float underlit = pow(1.0 - up, 3.0);
    float3 cloud_colour = lerp(float3(0.06, 0.045, 0.11), float3(0.38, 0.17, 0.36), underlit);
    sky = lerp(sky, cloud_colour, smoothstep(0.40, 0.78, cloud) * 0.85 * smoothstep(0.0, 0.06, up));

#if !ENV
    // A few stars show through the glow, high up and between the clouds.
    float3 grid = n * 120.0;
    float3 box = floor(grid);
    float3 centre = box + 0.5 + ((float3(Hash3(box), Hash3(box + 7.1), Hash3(box + 13.7)) - 0.5) * 0.5);
    float star = step(0.985, Hash3(box + 21.3)) * (1.0 - smoothstep(0.0, 0.16, length(grid - centre)));
    sky += star * 0.35 * smoothstep(0.35, 0.8, up) * (1.0 - smoothstep(0.35, 0.6, cloud)) * float3(0.85, 0.90, 1.0);
#endif

    // The wet street, dark, with a sheen of the city's glow toward the buildings.
    float3 street = float3(0.030, 0.024, 0.042) + (haze * 0.30 * exp(e / 0.04));
    sky = lerp(sky, street, smoothstep(0.0, 0.006, -e));

    // Distant rows along the horizon, seen between the near buildings: a far row lost in the
    // haze, a middle row and a nearer one. Below the street they are not worked out.
    [branch]
    if ((e > -0.02) && (e < 0.40))
    {
        [loop]
        for (int row = 0; row < 3; row++)
        {
            float cells = (row == 0) ? 230.0 : ((row == 1) ? 120.0 : 60.0);
            float base = (row == 0) ? -0.004 : ((row == 1) ? -0.008 : -0.012);
            float low = (row == 0) ? 0.006 : ((row == 1) ? 0.018 : 0.035);
            float high = (row == 0) ? 0.050 : ((row == 1) ? 0.15 : 0.30);
            float gap = (row == 0) ? 0.35 : ((row == 1) ? 0.7 : 1.0);
            float3 body = (row == 0) ? float3(0.10, 0.05, 0.13) : ((row == 1) ? float3(0.055, 0.035, 0.085) : float3(0.030, 0.026, 0.050));
            float2 pane = (row == 0) ? float2(0.0030, 0.0042) : ((row == 1) ? float2(0.0040, 0.0056) : float2(0.0055, 0.0075));
            float bright = (row == 0) ? 0.55 : ((row == 1) ? 0.70 : 0.80);
            float fog = (row == 0) ? 0.45 : ((row == 1) ? 0.25 : 0.12);
            float4 rows = Skyline(u + (0.0031 * row), e, pixel, clock, cells, base, low, high, gap, row + 1.0, body, pane, bright);
            sky = lerp(sky, lerp(rows.rgb, haze, fog), rows.a);
            if (row == 0)
            {
                sky = lerp(sky, haze, 0.30 * exp(-abs(e) / 0.025));
            }
        }
        sky = lerp(sky, haze, 0.16 * exp(-abs(e) / 0.020));
    }

    // The near city: a ring of buildings close round, and three more behind it, the nearest
    // first. The farther ring only shows where the nearer leaves a gap, and the light of its
    // signs only where it can be seen.
    float3 city = float3(0.0, 0.0, 0.0);
    float shown = 0.0;
    float3 glow = float3(0.0, 0.0, 0.0);
    SignHit front_sign = (SignHit)0;
    [loop]
    for (int ring = 0; ring < 4; ring++)
    {
        // Four rings, each farther, with more and taller buildings, so the gaps between the
        // near ones show more city behind them rather than sky. They stand close (brought in
        // by about 0.7 on 18 Nov 2026); the places round each ring keep the buildings their
        // width, so nearer means bigger on screen.
        float D = (ring == 0) ? 0.7 : ((ring == 1) ? 1.7 : ((ring == 2) ? 3.0 : 5.0));
        float cells = (ring == 0) ? 9.0 : ((ring == 1) ? 15.0 : ((ring == 2) ? 22.0 : 31.0));
        float low = (ring == 0) ? 0.25 : ((ring == 1) ? 0.45 : ((ring == 2) ? 0.9 : 1.4));
        float high = (ring == 0) ? 1.5 : ((ring == 1) ? 2.8 : ((ring == 2) ? 4.4 : 6.8));
        float3 light = float3(0.0, 0.0, 0.0);
        SignHit hit = (SignHit)0;
        float4 layer = Facade(phi, e, pixel, clock, D, cells, low, high, ring, light, hit);
        // Of the rings' signs at this point, the one that shows most is drawn after the loop;
        // the farther rings' signs a little dimmed by the air.
        hit.weight *= (1.0 - shown) * layer.a * (1.0 - (0.10 * ring));
        if (hit.weight > front_sign.weight)
        {
            front_sign = hit;
        }
        // The farther rings are more and more lost in the wet air.
        layer.rgb = lerp(layer.rgb, haze * 0.5, 0.14 * ring);
        glow += light * (1.0 - shown) * (1.0 - (0.12 * ring));
        city += (1.0 - shown) * layer.a * layer.rgb;
        shown += (1.0 - shown) * layer.a;
        if (shown > 0.995)
        {
            break;
        }
    }
#if !ENV
    [branch]
    if (front_sign.weight > 0.0)
    {
        city += front_sign.weight * SignColour(front_sign.sign, front_sign.p, front_sign.aa, clock);
    }
#endif
    sky = city + ((1.0 - shown) * sky) + glow;

#if !ENV
    // Utility poles and power lines strung across the street, nearest of all: dark against the
    // glow, catching a little of the neon.
    [branch]
    if (e > -0.05)
    {
        float wires = Wires(n, phi, e, pixel);
        // A faint sheen of the neon along them, so they show against the dark sky overhead too.
        sky = lerp(sky, float3(0.040, 0.032, 0.058) + (glow * 0.6), wires);
    }
#endif

#if !ENV
    // A very light rain: the calm rain's soft streaks, few and faint. Each streak is lit by
    // what is behind it, so against the dark sky it takes a little of the city's purple and in
    // front of the signs a share of their own light, and it shows faintly everywhere rather than
    // vanishing against anything bright.
    float2 screen = i.position.xy * G[50].xy;
    float rain = Downpour(screen, G[50].y / G[50].x, clock);
    float3 lit = lerp(float3(0.24, 0.18, 0.32), float3(0.18, 0.20, 0.30), up) + (sky * 0.55) + (glow * 0.8);
    sky += rain * lit * 0.55;
#endif
#if !ENV
    // A film's look: deeper shadows, richer colour.
    sky = pow(max(sky, 0.0), 1.10) * 1.08;
#endif
    return sky * LEVEL;
