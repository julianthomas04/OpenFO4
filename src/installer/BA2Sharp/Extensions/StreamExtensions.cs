// Modified for OpenFO4 in 2026, including provenance notices; GPL-3.0-only. See SOURCE.md.
using System;
using System.IO;

namespace SharpBSABA2.Extensions
{
    public static class StreamExtensions
    {
        public static byte[] ReadBytes(this Stream stream, long count)
        {
            using (var memoryStream = new MemoryStream())
            {
                long remainingBytes = count;

                while (remainingBytes > 0)
                {
                    // Determine how many bytes to read in this iteration
                    int bytesToRead = (int)Math.Min(remainingBytes, int.MaxValue);

                    // Create buffer for this iteration
                    byte[] buffer = new byte[bytesToRead];

                    // Read into the buffer
                    // Critically, we can pass 0 as offset because our buffer is sized exactly to bytesToRead
                    int bytesRead = stream.Read(buffer, 0, bytesToRead);

                    // Write to memory stream
                    memoryStream.Write(buffer, 0, bytesRead);

                    // Update remaining bytes
                    remainingBytes -= bytesRead;

                    // Break if no more bytes were read (end of stream)
                    if (bytesRead == 0) break;
                }

                return memoryStream.ToArray();
            }

        }
    }
}
