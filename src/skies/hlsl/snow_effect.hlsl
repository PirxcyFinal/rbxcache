    float clock = TIME;

    // A bright, pale overcast, with the ground below the horizon a snowy blue-white.
    float2 deck = (n.xz / (abs(n.y) + 0.35)) * 1.1;
    float cloud = saturate((Cloud(deck) * 0.7) + (Cloud((deck * 2.1) + 31.0) * 0.3));
    float3 sky = lerp(float3(0.30, 0.35, 0.42), float3(0.52, 0.57, 0.64), smoothstep(0.25, 0.75, cloud));
    float haze = pow(1.0 - saturate(abs(n.y)), 4.5);
    sky = lerp(sky, float3(0.56, 0.60, 0.66), haze * 0.55);
    sky = lerp(sky, float3(0.50, 0.55, 0.62), saturate(-n.y * 3.0) * 0.8);

#if !ENV
    float2 screen = i.position.xy * G[50].xy;
    float aspect = G[50].y / G[50].x;
    float2 u = float2(screen.x * aspect, screen.y);

    // Four sheets from near to far. Nearer flakes are larger and fewer and change more slowly;
    // far ones are fine and change a little faster. All are far slower than the storm's rain.
    float pixel = G[50].y;
    float snow = 0.0;
    snow += SnowSheet(u, pixel, clock, 0.0, 7.0, 5.0, SPEED * 1.3, 0.0058) * 1.00;
    snow += SnowSheet(u, pixel, clock, 1.0, 11.0, 8.0, SPEED * 1.6, 0.0040) * 0.85;
    snow += SnowSheet(u, pixel, clock, 2.0, 17.0, 12.0, SPEED * 2.0, 0.0028) * 0.70;
    snow += SnowSheet(u, pixel, clock, 3.0, 26.0, 18.0, SPEED * 2.4, 0.0019) * 0.55;
    // Only the very edge eases off, as in the rain, so there is no band where the snow stops.
    snow *= smoothstep(0.0, 0.05, screen.y) * (1.0 - smoothstep(0.92, 1.0, screen.y));
    snow = 1.0 - exp(-snow * AMOUNT);

    // Soft white over the cloud, a little fainter in the haze.
    sky = lerp(sky, float3(0.93, 0.95, 0.98), saturate(snow * (0.90 - (0.18 * haze))));
#endif
    return sky;
