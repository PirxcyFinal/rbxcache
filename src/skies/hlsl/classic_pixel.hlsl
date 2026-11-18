// The classic sky's pixel shaders (SkyFS, SkyLinearFS, SkyRGBEFS and the rest), as Roblox
// builds them, with the effect on the skybox. Games without an Atmosphere draw their sky with
// these, Rivals among them. RGBE, LINEAR and VCOLOR pick the variant.
cbuffer CB0 : register(b0) { float4 G[61]; };
Texture2D DiffuseMapTexture : register(t0);
SamplerState DiffuseMapSampler : register(s0);

// The engine's looping clock, 0 to 1: the last number of FogColor_GlobalForceFieldTime.
#define TIME (G[19].w)

struct SkyIn
{
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
    float4 colour : COLOR0;
    float3 direction : TEXCOORD1;
};

float3 sqr3(float3 x)
{
    return x * x;
}

@FUNCTIONS@

// `rgb` is the sky as it would be shown, `n` the direction this pixel looks in.
float3 Effect(float3 rgb, SkyIn i, float3 n)
{
    @EFFECT@
}

// 1 on the skybox, 0 on anything else these shaders draw. The same shaders put the sun and
// the moon up, as squares added over the sky, and an effect drawn on those would show as a
// lit square. A face of the skybox lies flat along two of the axes, so across it one of its
// three coordinates stays the same; the sun and moon face their own way, and all three change.
float SkyboxFace(float3 p)
{
    float3 change = abs(ddx(p)) + abs(ddy(p));
    float flat = min(change.x, min(change.y, change.z)) / max(length(p), 0.0001);
    return 1.0 - smoothstep(0.000015, 0.00006, flat);
}

float4 main(SkyIn i) : SV_Target
{
    float face = SkyboxFace(i.direction);
    float3 n = normalize(i.direction);
    float4 texel = DiffuseMapTexture.Sample(DiffuseMapSampler, i.uv);
#if VCOLOR
    float4 tint = i.colour;
#else
    float4 tint = float4(1.0, 1.0, 1.0, 1.0);
#endif
#if RGBE
    float a = texel.w * tint.w;
    float3 rgb = (a > 0.0) ? ((texel.xyz * tint.xyz) + 0.00196078442968428134918212890625) * exp2((floor(a * 255.0) * 0.125) - 16.0) : float3(0.0, 0.0, 0.0);
  #if LINEAR
    rgb = lerp(rgb, sqr3(Effect(sqrt(saturate(rgb)), i, n)), face);
  #else
    rgb = sqrt(saturate(rgb * G[20].y));
    rgb = lerp(rgb, Effect(rgb, i, n), face);
  #endif
    return float4(rgb, 1.0);
#else
    float4 c = texel * tint;
    float3 rgb = c.xyz * c.xyz;
  #if LINEAR
    rgb = lerp(rgb, sqr3(Effect(sqrt(saturate(rgb)), i, n)), face);
  #else
    rgb = sqrt(saturate(rgb * G[20].y));
    rgb = lerp(rgb, Effect(rgb, i, n), face);
  #endif
    return float4(rgb * c.w, c.w);
#endif
}
