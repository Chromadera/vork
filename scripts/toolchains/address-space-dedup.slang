RWStructuredBuffer<float> outputBuffer;
[noinline] float leaf(float values[19], uint index, out float results[15])
{
    for(uint i = 0; i < 15; ++i) results[i] = sin(values[(i + index) % 19]);
    return results[index % 15];
}
[noinline] float f(uint index)
{
    float values[19];
    for(uint i = 0; i < 19; ++i) values[i] = outputBuffer[i];
    float results[15];
    return leaf(values, index, results) + results[(index+1)%15];
}
[noinline] float g(uint index)
{
    float values[19];
    for(uint i = 0; i < 19; ++i) values[i] = outputBuffer[i+1];
    float results[15];
    return leaf(values, index, results) + results[(index+2)%15];
}
[shader("compute")]
[numthreads(1,1,1)]
void main(uint3 tid : SV_DispatchThreadID)
{
    outputBuffer[tid.x] = f(tid.x) + f(tid.y) + g(tid.z);
}
