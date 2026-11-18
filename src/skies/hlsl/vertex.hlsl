cbuffer CB0 : register(b0) { float4 G[61]; };
cbuffer CB1 : register(b1) { float4 P[22]; };

#define TIME (G[19].w)

struct VertexIn
{
    float3 position : POSITION;
    float2 uv : TEXCOORD0;
    float4 unused : TEXCOORD1;
};

struct VertexOut
{
    float4 position : SV_Position;
    float3 direction : TEXCOORD0;
    float2 uv : TEXCOORD1;
    float4 colour : TEXCOORD2;
};

float3 Move(float3 position)
{
    @MOVE@
}

VertexOut main(VertexIn i)
{
    float4 local = float4(Move(i.position), 1.0);
    float4 world = float4(dot(P[0], local), dot(P[1], local), dot(P[2], local), 1.0);
    VertexOut o;
    o.position = float4(dot(P[4], world), dot(P[5], world), 0.0, dot(P[7], world));
    o.direction = world.xyz;
    o.uv = i.uv;
    o.colour = lerp(P[13], P[12], saturate(world.y * 0.0005882352706976234912872314453125));
    return o;
}
