#if ENV
    float clock = 0.5;   // reflections get a still picture
#else
    float clock = saturate(TIME);
#endif
    // Same crest compensation as the rain, folded back into 0..1 so the frames stay in range.
    // Playback runs quicker over the crest of a big swell and still retraces when the clock does.
    float c = min(clock, 0.999);
    float along = ((0.65 * asin(c)) / 1.5707963) + ((0.35 * log((1.0 + c) / (1.0 - c))) / 7.600902);
    float spot = along * (FRAMES - 1.0);
    float first = min(floor(spot), FRAMES - 2.0);
    float blend = saturate(spot - first);

    float2 at = float2((atan2(n.z, n.x) / 6.2831853) + 0.5, 1.0 - (asin(saturate(n.y)) / radians(TOP)));
    float3 picture = float3(0.0, 0.0, 0.0);
    // Only the part of the sky the picture covers reads the table, and the next frame is
    // only read while it is actually being blended in.
#if !ENV
    // (Reflections skip the picture entirely, which keeps their shader small and quick.)
    [branch] if (n.y > 0.0 && at.y > 0.0 && at.y < 1.0)
    {
        picture = Frame(uint(first), at);
        [branch] if (blend > 0.02)
        {
            picture = lerp(picture, Frame(uint(first) + 1, at), blend);
        }
        picture *= smoothstep(0.0, 0.06, at.y);
    }
#endif

    // The game's own sky, dimmed to night, with the picture added on top.
    float3 night = lerp(float3(0.010, 0.018, 0.045), float3(0.030, 0.050, 0.100), pow(1.0 - saturate(abs(n.y)), 3.0));
    return (rgb * DIM) + (night * (1.0 - DIM)) + (picture * GLOW);
