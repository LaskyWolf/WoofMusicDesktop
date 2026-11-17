// Liquid Glass: refracts the content drawn behind a rounded rectangle.
// glass.rs copies that content into `backdrop` before this pass runs.

struct Uniforms {
    rect: vec4<f32>,   // glass rect in the window, physical px: x, y, width, height
    region: vec4<f32>, // where `backdrop` was copied from, physical px: x, y, width, height
    params: vec4<f32>, // window width, window height, corner radius (px), scale factor
    tint: vec4<f32>,   // tint colour, alpha = strength
    flags: vec4<f32>,  // x: 1 when `backdrop` holds the content behind, 0 when it could not be copied
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var backdrop: texture_2d<f32>;
@group(0) @binding(2) var backdrop_sampler: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let px = u.rect.xy + corners[i] * u.rect.zw;
    return vec4(px.x / u.params.x * 2.0 - 1.0, 1.0 - px.y / u.params.y * 2.0, 0.0, 1.0);
}

// Signed distance to a rounded rectangle centred at the origin (negative inside).
fn sd_round_rect(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half_size + r;
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sample_at(px: vec2<f32>) -> vec3<f32> {
    let uv = clamp((px - u.region.xy) / u.region.zw, vec2(0.0), vec2(1.0));
    return textureSampleLevel(backdrop, backdrop_sampler, uv, 0.0).rgb;
}

// Small disk blur: frosts the glass without hiding what is behind it.
fn frosted(px: vec2<f32>, radius: f32) -> vec3<f32> {
    var sum = sample_at(px);
    let taps = 12;
    for (var i = 0; i < taps; i++) {
        let a = f32(i) * 2.39996; // golden angle spiral
        let r = radius * sqrt((f32(i) + 0.5) / f32(taps));
        sum += sample_at(px + vec2(cos(a), sin(a)) * r);
    }
    return sum / f32(taps + 1);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let px = pos.xy;
    let scale = u.params.w;
    let half_size = u.rect.zw * 0.5;
    let local = px - (u.rect.xy + half_size);
    let r = min(u.params.z, min(half_size.x, half_size.y));
    let d = sd_round_rect(local, half_size, r);
    let coverage = clamp(0.5 - d, 0.0, 1.0); // antialiased edge
    if coverage <= 0.0 {
        discard;
    }

    // Outward surface normal from the distance field.
    let e = vec2(1.0, 0.0);
    let n = normalize(vec2(
        sd_round_rect(local + e.xy, half_size, r) - sd_round_rect(local - e.xy, half_size, r),
        sd_round_rect(local + e.yx, half_size, r) - sd_round_rect(local - e.yx, half_size, r),
    ) + 1e-5);

    // 0 at the edge, 1 once `bevel` px inside: the curved rim of the glass.
    let bevel = min(r, 18.0 * scale);
    let depth = clamp(-d / bevel, 0.0, 1.0);
    let curve = 1.0 - sqrt(1.0 - (1.0 - depth) * (1.0 - depth)); // circular profile

    var col: vec3<f32>;
    var alpha = 1.0;
    if u.flags.x > 0.5 {
        // Refraction: the rim bends light from just outside the glass inward,
        // with a colour split that grows with the bend, like a lens edge.
        let bend = curve * 22.0 * scale;
        let base = px + n * bend;
        let blur = 3.0 * scale;
        col = vec3(
            frosted(base + n * bend * 0.06, blur).r,
            frosted(base, blur).g,
            frosted(base - n * bend * 0.06, blur).b,
        );
        let luma = dot(col, vec3(0.2126, 0.7152, 0.0722));
        col = mix(vec3(luma), col, 1.25); // slight saturation lift
        col = mix(col, u.tint.rgb, u.tint.a);
    } else {
        // No backdrop on this GPU: a translucent tint, still with the rim.
        col = u.tint.rgb;
        alpha = 0.82;
    }

    // Specular rim: light from the top left, a fainter bounce bottom right.
    let light = normalize(vec2(-0.6, -0.8));
    let rim = smoothstep(1.6 * scale, 0.0, -d);
    let key = pow(max(dot(n, light), 0.0), 2.0);
    let fill = pow(max(dot(n, -light), 0.0), 3.0) * 0.45;
    col += vec3(1.0) * rim * (0.10 + 0.55 * (key + fill));
    col += vec3(1.0) * curve * 0.06 * (key + fill);

    let a = alpha * coverage;
    return vec4(clamp(col, vec3(0.0), vec3(1.0)) * a, a);
}
