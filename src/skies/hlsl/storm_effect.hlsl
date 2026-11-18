    float clock = TIME;

    // Heavy, dark cloud deck: big billows with wisps over them, nearly black overhead.
    float2 deck = (n.xz / (abs(n.y) + 0.30)) * 1.4;
    float cloud = saturate((Cloud(deck) * 0.75) + (Cloud((deck * 2.3) + 40.0) * 0.25));
    float3 sky = lerp(float3(0.035, 0.042, 0.055), float3(0.18, 0.20, 0.24), smoothstep(0.30, 0.74, cloud));
    float haze = pow(1.0 - saturate(abs(n.y)), 5.0);
    sky = lerp(sky, float3(0.22, 0.24, 0.28), haze * 0.65);

    float flash = 0.0;
#if !ENV
    // Where lightning strikes depends on where the camera is, so it is not the same bolt all over the map.
    float2 place = floor(G[11].xz / 48.0);
    float seed = Hash2(place);
    float bearing = seed * 6.2831853;
    float3 lit = float3(0.70, 0.78, 1.0);

    // Strikes are spread through the whole cycle. Parking them at the top of the clock's swell
    // made the storm look as if it stopped and started again each time the clock turned around.
    // About two strikes every three seconds, never silent for more than a few seconds, dimmer
    // while the clock's pulse is small.
    // Same pace as the rain. 1.34 keeps the old strike rate when the clock reads 0.5.
    float drive = Pace(clock) * 1.34;
    float beat = floor(drive + seed);
    float age = frac(drive + seed);
    float pick = step(0.55, Hash1(beat + 4.2));
    float portion = saturate(age / 0.45);
    float swell = sin(3.14159265 * portion);
    swell *= swell;
    flash = pick * swell * (0.45 + (0.55 * smoothstep(0.35, 0.90, clock))) * LIGHTNING;
    float3 toward = normalize(float3(cos(bearing), 0.35, sin(bearing)));
    sky += flash * (0.03 + (0.22 * pow(saturate(dot(n, toward)), 5.0))) * (0.30 + cloud) * lit;

    // The bolt: a jagged line from the cloud down toward the horizon.
    float kink = seed * 50.0;
    float path = bearing + (0.16 * (Noise2(float2(n.y * 7.0, 3.3 + kink)) - 0.5)) + (0.07 * (Noise2(float2(n.y * 23.0, 8.1 + kink)) - 0.5))
        + (0.025 * (Noise2(float2(n.y * 71.0, 5.7 + kink)) - 0.5));
    float off = atan2(n.z, n.x) - path;
    off -= 6.2831853 * round(off / 6.2831853);
    float away = abs(off) * sqrt(saturate(1.0 - (n.y * n.y)));
    float reach = step(0.0, n.y) * (1.0 - smoothstep(0.42, 0.62, n.y));
    float bolt = ((1.0 - smoothstep(0.0008, 0.0035, away)) + (0.22 * (1.0 - smoothstep(0.0, 0.03, away)))) * reach;
    sky += bolt * pick * swell * LIGHTNING * float3(0.80, 0.85, 1.0);

    // A second, rarer flash in another part of the sky, on its own picks of the same beats.
    float3 distant = normalize(float3(cos(bearing + 2.4), 0.25, sin(bearing + 2.4)));
    sky += step(0.82, Hash1(beat + 9.1)) * swell * 0.35 * LIGHTNING * pow(saturate(dot(n, distant)), 6.0) * (0.30 + cloud) * lit;

    float2 screen = i.position.xy * G[50].xy;
    float rain = Downpour(screen, G[50].y / G[50].x, clock);
    sky += rain * float3(0.45, 0.50, 0.58) * (0.50 + (0.35 * flash)) * (0.85 - (0.30 * haze));
    // Spray and mist hanging over the horizon.
    sky = lerp(sky, float3(0.26, 0.29, 0.33), haze * 0.10);
#endif
    return sky;
