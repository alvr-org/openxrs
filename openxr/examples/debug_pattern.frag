#version 450

layout(location = 0) in vec2 screen_coords;
layout(location = 0) out vec4 color;

void main() {
    color = vec4(screen_coords, 0, 1);
}
