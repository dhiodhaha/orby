// Liquid metal, ported to HLSL from Paper Shaders `liquidMetal` (Apache-2.0, see NOTICE).
// Only the non-image path with the circle shape is kept: that is the mercury drop.
// uv is 0..1 across the orb square, y pointing down, the same as the GLSL original.

cbuffer Params : register(b0)
{
    float2 orbOrigin;   // px, top-left of the orb square in the swap chain
    float  orbSize;     // px
    float  time;        // shader time (still frame at rest)
    float  angle;       // highlight angle in degrees (90 = noon)
    float  distortion;
    float  opacity;     // whole-orb opacity (tuck = 0.4)
    float  brightness;  // 1.0 by day, 0.7 after Isya
    float  softness;
    float  repetition;
    float  shiftRed;
    float  shiftBlue;
    float  contour;
    float  tintA;
    float  ringStart;    // 0..1 of a turn: where the arc begins (the travelling arc while reading a link)
    float  pad1;
    float  ringMode;     // 0 none, 1 progress (hairline track + arc), 2 focus (dotted track + arc)
    float  ringProgress; // 0..1, clockwise from 12 o'clock
    float  ringRadius;   // px
    float  px1;          // one logical pixel in px
    float4 ringInk;      // premultiplied
    float4 ringTrack;    // premultiplied
    float4 ringHalo;     // premultiplied, opposite colour so the ring reads over any wallpaper
};

// Coverage of a stroke of half-width hw at distance dr, with a 1 px soft edge.
float cover(float dr, float hw) { return 1.0 - smoothstep(hw - 0.5, hw + 0.5, dr); }

float4 over(float4 top, float4 under) { return top + under * (1.0 - top.a); }

// The ring around the drop. Returns a premultiplied colour.
float4 ring(float2 p)
{
    const float PI = 3.14159265358979323846;
    if (ringMode < 0.5) return float4(0, 0, 0, 0);
    float2 c = orbOrigin + orbSize * 0.5;
    float2 d = p - c;
    float dist = length(d);
    float a01 = frac(atan2(d.x, -d.y) / (2.0 * PI) + 1.0);
    float dr = abs(dist - ringRadius);
    float halo = 0.75 * px1; // the halo reaches 0.75 px past every mark
    if (frac(a01 - ringStart + 1.0) <= ringProgress)
    {
        float hw = 1.0 * px1; // 2 px arc
        return over(ringInk * cover(dr, hw), ringHalo * cover(dr, hw + halo));
    }
    if (ringMode < 1.5)
    {
        float hw = 0.75 * px1; // 1.5 px hairline track
        return over(ringTrack * cover(dr, hw), ringHalo * 0.6 * cover(dr, hw + halo));
    }
    // Dotted track: 1.5 px dots every 6 px of arc.
    float circ = 2.0 * PI * ringRadius;
    float n = max(1.0, floor(circ / (6.0 * px1)));
    float s = (frac(a01 * n) - 0.5) * (circ / n);
    float r = sqrt(s * s + dr * dr);
    return over(ringTrack * cover(r, 0.75 * px1), ringHalo * cover(r, 0.75 * px1 + halo));
}

static const float PI = 3.14159265358979323846;

struct VSOut { float4 pos : SV_Position; };

VSOut vs_main(uint id : SV_VertexID)
{
    // One triangle that covers the whole target.
    float2 p = float2((id << 1) & 2, id & 2);
    VSOut o;
    o.pos = float4(p * float2(2, -2) + float2(-1, 1), 0, 1);
    return o;
}

float2 mod289(float2 x) { return x - 289.0 * floor(x / 289.0); }
float3 mod289(float3 x) { return x - 289.0 * floor(x / 289.0); }
float3 permute(float3 x) { return mod289(((x * 34.0) + 1.0) * x); }

float snoise(float2 v)
{
    const float4 C = float4(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
    float2 i = floor(v + dot(v, C.yy));
    float2 x0 = v - i + dot(i, C.xx);
    float2 i1 = (x0.x > x0.y) ? float2(1.0, 0.0) : float2(0.0, 1.0);
    float4 x12 = x0.xyxy + C.xxzz;
    x12.xy -= i1;
    i = mod289(i);
    float3 p = permute(permute(i.y + float3(0.0, i1.y, 1.0)) + i.x + float3(0.0, i1.x, 1.0));
    float3 m = max(0.5 - float3(dot(x0, x0), dot(x12.xy, x12.xy), dot(x12.zw, x12.zw)), 0.0);
    m = m * m;
    m = m * m;
    float3 x = 2.0 * frac(p * C.www) - 1.0;
    float3 h = abs(x) - 0.5;
    float3 ox = floor(x + 0.5);
    float3 a0 = x - ox;
    m *= 1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h);
    float3 g;
    g.x = a0.x * x0.x + h.x * x0.y;
    g.yz = a0.yz * x12.xz + h.yz * x12.yw;
    return 130.0 * dot(m, g);
}

float2 rotate2(float2 uv, float th)
{
    float c = cos(th), s = sin(th);
    return float2(c * uv.x - s * uv.y, s * uv.x + c * uv.y);
}

float colorChanges(float c1, float c2, float stripe, float3 w, float blur, float bump, float tint)
{
    float ch = lerp(c2, c1, smoothstep(0.0, 2.0 * blur, stripe));
    float border = w.x;
    ch = lerp(ch, c2, smoothstep(border, border + 2.0 * blur, stripe));
    border = w.x + 0.4 * (1.0 - bump) * w.y;
    ch = lerp(ch, c1, smoothstep(border, border + 2.0 * blur, stripe));
    border = w.x + 0.5 * (1.0 - bump) * w.y;
    ch = lerp(ch, c2, smoothstep(border, border + 2.0 * blur, stripe));
    border = w.x + w.y;
    ch = lerp(ch, c1, smoothstep(border, border + 2.0 * blur, stripe));
    float gt = (stripe - w.x - w.y) / w.z;
    float gradient = lerp(c1, c2, smoothstep(0.0, 1.0, gt));
    ch = lerp(ch, gradient, smoothstep(border, border + 0.5 * blur, stripe));
    // Tint by colour burn. The tint is white, so this keeps the metal untinted.
    ch = lerp(ch, 1.0 - min(1.0, (1.0 - ch) / max(tint, 0.0001)), tintA);
    return ch;
}

float4 ps_main(VSOut i) : SV_Target
{
    float2 uv = (i.pos.xy - orbOrigin) / orbSize;
    float t = 0.3 * (time + 2.8);
    float cycleWidth = repetition;

    float2 rotatedUV = uv - 0.5;
    float a = (-angle + 70.0) * PI / 180.0;
    float cosA = cos(a), sinA = sin(a);
    rotatedUV = float2(rotatedUV.x * cosA - rotatedUV.y * sinA, rotatedUV.x * sinA + rotatedUV.y * cosA) + 0.5;

    // Circle shape.
    float2 shapeUV = (uv - 0.5) * 0.67;
    float edge = pow(saturate(3.0 * length(shapeUV)), 18.0);
    edge = lerp(smoothstep(0.9 - 2.0 * fwidth(edge), 0.9, edge), edge, smoothstep(0.0, 0.4, contour));
    float alpha = 1.0 - smoothstep(0.9 - 2.0 * fwidth(edge), 0.9, edge);
    edge = 1.2 * edge;
    float4 rc = ring(i.pos.xy) * opacity;
    if (alpha <= 0.0) return rc;

    float uy = saturate(uv.y);
    float diagBLtoTR = rotatedUV.x - rotatedUV.y;
    float diagTLtoBR = rotatedUV.x + rotatedUV.y;
    float3 color1 = float3(0.98, 0.98, 1.0);
    float3 color2 = float3(0.1, 0.1, 0.1 + 0.1 * smoothstep(0.7, 1.3, diagTLtoBR));

    float2 gradUV = uv - 0.5;
    float dist = length(gradUV + float2(0.0, 0.2 * diagBLtoTR));
    gradUV = rotate2(gradUV, (0.25 - 0.2 * diagBLtoTR) * PI);
    float direction = gradUV.x;

    float bump = pow(max(1.8 * dist, 0.0), 1.2);
    bump = 1.0 - bump;
    bump *= pow(uy, 0.3);

    float thin1 = 0.12 / cycleWidth * (1.0 - 0.4 * bump);
    float thin2 = 0.07 / cycleWidth * (1.0 + 0.4 * bump);
    float wide = 1.0 - thin1 - thin2;
    float thin1w = cycleWidth * thin1;
    float thin2w = cycleWidth * thin2;

    float noise = snoise(uv - t);
    edge += (1.0 - edge) * distortion * noise;

    direction += diagBLtoTR;
    float contourV = 0.0;
    direction -= 2.0 * noise * diagBLtoTR * (smoothstep(0.0, 1.0, edge) * (1.0 - smoothstep(0.0, 1.0, edge)));
    direction *= lerp(1.0, 1.0 - edge, smoothstep(0.5, 1.0, contour));
    direction -= 1.7 * edge * smoothstep(0.5, 1.0, contour);
    direction += 0.2 * pow(contour, 4.0) * (1.0 - smoothstep(0.0, 1.0, edge));

    bump *= clamp(pow(uy, 0.1), 0.3, 1.0);
    direction *= (0.1 + (1.1 - edge) * bump);
    direction *= (0.4 + 0.6 * (1.0 - smoothstep(0.5, 1.0, edge)));
    direction += 0.18 * (smoothstep(0.1, 0.2, uy) * (1.0 - smoothstep(0.2, 0.4, uy)));
    direction += 0.03 * (smoothstep(0.1, 0.2, 1.0 - uy) * (1.0 - smoothstep(0.2, 0.4, 1.0 - uy)));
    direction *= (0.5 + 0.5 * pow(uy, 2.0));
    direction *= cycleWidth;
    direction -= t;

    float colorDispersion = saturate(1.0 - bump);
    float dR = colorDispersion;
    dR += 0.03 * bump * noise;
    dR += 5.0 * (smoothstep(-0.1, 0.2, uy) * (1.0 - smoothstep(0.1, 0.5, uy))) * (smoothstep(0.4, 0.6, bump) * (1.0 - smoothstep(0.4, 1.0, bump)));
    dR -= diagBLtoTR;
    float dB = colorDispersion * 1.3;
    dB += (smoothstep(0.0, 0.4, uy) * (1.0 - smoothstep(0.1, 0.8, uy))) * (smoothstep(0.4, 0.6, bump) * (1.0 - smoothstep(0.4, 0.8, bump)));
    dB -= 0.2 * edge;
    dR *= shiftRed / 20.0;
    dB *= shiftBlue / 20.0;

    float blur = softness / 15.0 + 0.3 * contourV;
    float3 w = float3(thin1w, thin2w, wide);
    w.y -= 0.02 * smoothstep(0.0, 1.0, edge + bump);
    float sR = frac(direction + dR);
    float r = colorChanges(color1.r, color2.r, sR, w, blur + fwidth(sR), bump, 1.0);
    float sG = frac(direction);
    float g = colorChanges(color1.g, color2.g, sG, w, blur + fwidth(sG), bump, 1.0);
    float sB = frac(direction - dB);
    float b = colorChanges(color1.b, color2.b, sB, w, blur + fwidth(sB), bump, 1.0);

    float3 color = float3(r, g, b) * brightness;
    // Dither against banding, kept inside the shape so the output stays premultiplied.
    color += (1.0 / 256.0) * (frac(sin(dot(0.014 * i.pos.xy, float2(12.9898, 78.233))) * 43758.5453123) - 0.5);
    float outA = alpha * opacity;
    float4 orb = float4(saturate(color) * outA, outA);
    return orb + rc * (1.0 - outA);
}
