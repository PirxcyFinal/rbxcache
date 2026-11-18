cbuffer CB0 : register(b0) { float4 G[61]; };
cbuffer CB1 : register(b1) { float4 P[22]; };
Texture2D DiffuseMapTexture : register(t0);
SamplerState DiffuseMapSampler : register(s0);

// The engine's looping clock, 0 to 1: the last number of FogColor_GlobalForceFieldTime.
#define TIME (G[19].w)

struct SkyIn
{
    float4 position : SV_Position;
    float3 direction : TEXCOORD0;
    float2 uv : TEXCOORD1;
    float4 colour : TEXCOORD2;
};

@FUNCTIONS@

// `rgb` is the sky as it would be shown, `n` the direction this pixel looks in.
float3 Effect(float3 rgb, SkyIn i, float3 n)
{
    @EFFECT@
}

float4 main(SkyIn i) : SV_Target
{
    float4 texel = DiffuseMapTexture.Sample(DiffuseMapSampler, i.uv);
#if RGBE
    float exponent = texel.w;
    float3 decoded = (texel.xyz + 0.00196078442968428134918212890625) * exp2((floor(exponent * 255.0) * 0.125) - 16.0);
    float4 c = float4(exponent > 0.0 ? decoded : float3(0.0, 0.0, 0.0), 1.0) * i.colour;
#else
    float4 c = texel * i.colour;
    c.xyz = c.xyz * c.xyz;
#endif
    float3 n = normalize(i.direction);
    float f = exp2(P[14].x / (0.001000000047497451305389404296875 + pow(max(n.y, 9.9999997473787516355514526367188e-06), P[14].y)));
    float3 up = saturate(n);
    up = up * up;
    float3 down = saturate(-n);
    down = down * down;
    float fs = saturate(f);
    float3 horizon = ((P[16] * up.x) + (P[17] * down.x) + (P[18] * up.y) + (P[19] * down.y) + (P[20] * up.z) + (P[21] * down.z)).xyz * G[19].xyz
        + (G[15].xyz * (saturate(pow(float3(1.0, 1.0, 1.0) * (0.5 + (dot(n, -G[16].xyz) * 0.5)), P[15].xyz) * (1.0 - f)) * P[15].w));
    float3 rgb = lerp(horizon, c.xyz, fs);
#if LINEAR
  #if HAS_EFFECT
    // This variant hands on light, not a finished colour; the effect works on the colour.
    float3 shown = Effect(sqrt(saturate(rgb)), i, n);
    rgb = shown * shown;
  #endif
#else
    rgb = sqrt(saturate(rgb * G[20].y));
    rgb = Effect(rgb, i, n);
#endif
    return float4(rgb * c.w, max(1.0 - fs, step(0.5, min(texel.x, texel.w))));
}
