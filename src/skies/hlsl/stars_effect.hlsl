#if ENV
    float clock = 0.5;   // reflections get a still picture
#else
    float clock = saturate(TIME);
#endif
    float phi = atan2(n.z, n.x);            // round the horizon, in radians
    float e = asin(clamp(n.y, -1.0, 1.0));  // elevation, in radians
    // How far one screen pixel turns the view, in radians.
    float pixel = max(length(fwidth(n)), 0.0002);
    float up = saturate(n.y);

    // A clear blue night: deep navy overhead, lighter and bluer toward the horizon.
    float3 sky = lerp(float3(0.075, 0.16, 0.34), float3(0.028, 0.065, 0.17), smoothstep(0.0, 0.32, up));
    sky = lerp(sky, float3(0.008, 0.018, 0.062), smoothstep(0.22, 1.0, up));

    // The Milky Way: a band of faint light across the sky with darker lanes of dust down it.
    float3 pole = normalize(float3(0.42, 0.50, 0.76));
    float lat = asin(clamp(dot(n, pole), -1.0, 1.0));
    float band = exp(-(lat * lat) / (0.22 * 0.22));
    float clouds = Cloud3(n * 4.0);
    float galaxy = band * saturate((clouds * 1.3) - 0.15) * (0.75 + (0.5 * Cloud3((n * 12.0) + 7.0)));
    float dust = smoothstep(0.42, 0.68, Cloud3((n * 6.0) + 3.0)) * exp(-(lat * lat) / (0.08 * 0.08));
    galaxy *= 1.0 - (0.75 * dust);
    // Faint near the horizon, where there is more air to look through.
    float clear = smoothstep(0.02, 0.30, up);
    sky += galaxy * lerp(float3(0.10, 0.15, 0.28), float3(0.20, 0.20, 0.26), clouds) * MILKY * clear;

#if !ENV
    // Stars in three layers: many faint ones, fewer brighter ones, and a scatter of bright ones
    // with a glow. More of the faint ones crowd along the Milky Way.
    float3 stars = Stars(phi, e, pixel, clock, 0.011, 0.10 + (0.22 * band * MILKY), 0.45, 1.0);
    stars += Stars(phi, e, pixel, clock, 0.026, 0.22, 0.75, 2.0);
    stars += Stars(phi, e, pixel, clock, 0.070, 0.28, 1.10, 3.0);
    sky += stars * STARS * smoothstep(0.0, 0.20, up);

    // The shooting stars, each on its own path and its own stretch of the clock. METEORS 1 is
    // three of them round the sky, which on Roblox's clock shows one in view every few seconds.
    float count = clamp(round(3.0 * METEORS), 0.0, 10.0);
    float3 meteors = float3(0.0, 0.0, 0.0);
    for (int k = 0; k < 10; k++)
    {
        meteors += Meteor(n, pixel, clock, k, max(count, 1.0)) * step(k + 0.5, count);
    }
    sky += meteors * smoothstep(0.0, 0.08, up);
#endif

    // Hills along the horizon, a far blue ridge and a dark near one. They follow the horizon
    // all the way round without a seam.
    float3 around = float3(cos(phi), sin(phi), 0.0);
    float far_ridge = 0.025 + (0.075 * Cloud3((around * 2.2) + float3(0.0, 0.0, 5.5)));
    float near_ridge = -0.005 + (0.060 * Cloud3((around * 3.4) + float3(0.0, 0.0, 9.1)));
    float3 glow = float3(0.09, 0.18, 0.36);
    sky = lerp(sky, lerp(float3(0.040, 0.075, 0.150), glow, 0.25), smoothstep(pixel, -pixel, e - far_ridge));
    sky = lerp(sky, float3(0.012, 0.022, 0.048), smoothstep(pixel, -pixel, e - near_ridge));
    return sky * LEVEL;
