// The server address someone types behind "Self-hosted?": https:// is added
// when they leave the scheme off, and an http:// address stays as typed.
import 'package:flutter_test/flutter_test.dart';

import 'package:passbundle/sign_in.dart';

void main() {
  test('adds https:// when the scheme is left off', () {
    expect(serverAddress('test.passbundle.com'), 'https://test.passbundle.com');
    expect(
      serverAddress('  vault.example.com/  '),
      'https://vault.example.com/',
    );
    expect(
      serverAddress('vault.example.com:8443'),
      'https://vault.example.com:8443',
    );
    expect(serverAddress('192.168.1.20'), 'https://192.168.1.20');
  });

  test('keeps an address that names its scheme', () {
    expect(
      serverAddress('https://vault.example.com'),
      'https://vault.example.com',
    );
    expect(
      serverAddress('HTTPS://vault.example.com'),
      'https://vault.example.com',
    );
    // The core refuses it and says why.
    expect(
      serverAddress('http://vault.example.com'),
      'http://vault.example.com',
    );
  });

  test('leaves an empty field empty', () {
    expect(serverAddress('   '), '');
  });
}
