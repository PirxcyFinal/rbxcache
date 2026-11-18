static const float TAU = 6.2831853;

// Per curtain: height of its foot (0 horizon, 1 top of the picture), how far the foot wanders,
// how far the light reaches above the foot, and brightness.
static const float4 SHAPE[3] = { float4(0.16, 0.060, 0.13, 0.95), float4(0.27, 0.070, 0.18, 0.75), float4(0.38, 0.055, 0.23, 0.55) };
static const float4 PHASE_A[3] = { float4(0.7, 2.9, 5.1, 1.3), float4(3.8, 0.4, 2.2, 4.6), float4(5.6, 1.9, 3.3, 0.2) };
static const float4 PHASE_B[3] = { float4(4.1, 0.9, 2.7, 5.8), float4(1.5, 3.6, 5.0, 0.6), float4(2.4, 4.9, 0.3, 3.1) };

float Hash3(float3 p) { return frac(sin(dot(p, float3(127.1, 311.7, 74.7))) * 43758.5453); }
