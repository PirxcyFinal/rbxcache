    float clock = TIME;

    // A flat grey overcast, lighter and hazier toward the horizon.
    float2 deck = (n.xz / (abs(n.y) + 0.30)) * 1.2;
    float cloud = saturate((Cloud(deck) * 0.7) + (Cloud((deck * 2.3) + 40.0) * 0.3));
    float3 sky = lerp(float3(0.14, 0.16, 0.19), float3(0.34, 0.37, 0.42), smoothstep(0.25, 0.80, cloud));
    float haze = pow(1.0 - saturate(abs(n.y)), 4.0);
    sky = lerp(sky, float3(0.40, 0.43, 0.47), haze * 0.60);
    sky *= DARK;

#if !ENV
    // The storm's rain, sheet for sheet.
    float2 screen = i.position.xy * G[50].xy;
    float rain = Downpour(screen, G[50].y / G[50].x, clock);

    // Rain shows lighter than the cloud behind it and fades into the haze; no glow. This
    // overcast is paler than the storm's, so the streaks are lifted further to stand out as much.
    sky += rain * float3(0.50, 0.55, 0.62) * (0.66 - (0.30 * haze));
    // Fine mist near the horizon.
    sky = lerp(sky, float3(0.40, 0.43, 0.47), haze * 0.08 * MIST);
#endif
    return sky;
