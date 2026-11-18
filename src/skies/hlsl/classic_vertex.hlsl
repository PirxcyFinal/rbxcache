// The classic sky's vertex shader (SkyVS), as Roblox builds it, with the direction each vertex
// looks in handed on as well: the classic sky draws a skybox without an Atmosphere, and its
// pixel shader is given no direction of its own.
cbuffer CB0 : register(b0) { float4 G[61]; };
// World (0 to 3), the two view-projections (4 to 11), then the colour at the top and at the
// bottom of the sky.
cbuffer CB1 : register(b1) { float4 P[14]; };

#define TIME (G[19].w)

struct VertexIn
{
    float4 position : POSITION;
    float2 uv : TEXCOORD0;
    float4 colour : COLOR0;
};

struct VertexOut
{
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
    float4 colour : COLOR0;
    float3 direction : TEXCOORD1;
};

float3 Move(float3 position)
{
    @MOVE@
}

VertexOut main(VertexIn i)
{
    float4 local = float4(Move(i.position.xyz), i.position.w);
    float4 world = float4(dot(P[0], local), dot(P[1], local), dot(P[2], local), dot(P[3], local));
    VertexOut o;
    o.position = float4(dot(P[4], world), dot(P[5], world), 0.0, dot(P[7], world));
    o.uv = i.uv;
    o.colour = lerp(P[13], P[12], saturate(world.y * 0.0005882352706976234912872314453125)) * i.colour;
    o.direction = world.xyz;
    return o;
}
