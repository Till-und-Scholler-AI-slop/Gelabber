// Optional, identical Chrome source control. These hints are not a CBR guarantee.
export function fixtureCodecOptions(bitrate, enabled) {
  if (!enabled) return {};
  if (!Number.isSafeInteger(bitrate) || bitrate <= 0 || bitrate % 1000) throw new Error('Fixed video fixture requires whole kbit/s');
  const kbps = bitrate / 1000;
  return { videoGoogleMinBitrate: kbps, videoGoogleStartBitrate: kbps, videoGoogleMaxBitrate: kbps };
}

export function fixtureDescription(description, bitrate, enabled) {
  const options = fixtureCodecOptions(bitrate, enabled);
  if (!enabled) return description;
  const settings = `x-google-min-bitrate=${options.videoGoogleMinBitrate};x-google-start-bitrate=${options.videoGoogleStartBitrate};x-google-max-bitrate=${options.videoGoogleMaxBitrate}`;
  const eol = description.sdp.includes('\r\n') ? '\r\n' : '\n';
  const sdp = description.sdp.split(/(?=^m=)/m).map(section => {
    if (!section.startsWith('m=video ')) return section;
    const lines = section.split(eol);
    const payloads = lines.filter(line => /^a=rtpmap:\d+ VP8\//i.test(line)).map(line => line.match(/^a=rtpmap:(\d+)/)[1]);
    for (const payload of payloads) {
      const indices = lines.flatMap((line, index) => new RegExp(`^a=fmtp:${payload}(?:\\s|$)`).test(line) ? [index] : []);
      if (indices.length > 1) throw new Error('Ambiguous VP8 fmtp in fixture description');
      if (indices.length) {
        const index = indices[0];
        const remaining = lines[index].replace(/^a=fmtp:\d+\s*/, '').split(';').filter(value => value.trim() && !/^x-google-(min|start|max)-bitrate$/i.test(value.split('=')[0].trim()));
        lines[index] = `a=fmtp:${payload} ${[...remaining, settings].join(';')}`;
      } else {
        const index = lines.findIndex(line => line.startsWith(`a=rtpmap:${payload} `));
        lines.splice(index + 1, 0, `a=fmtp:${payload} ${settings}`);
      }
    }
    return lines.join(eol);
  }).join('');
  return { type: description.type, sdp };
}
