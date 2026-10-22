package com.passbundle.autofilltest

import android.app.Activity
import android.os.Bundle
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient

/**
 * A sign-in page in the test app's own WebView (an in-app browser), for the
 * autofill test: Android's autofill reads a web page's form (its fields'
 * HTML attributes, some with no value) and gets the page's domain (the `site`
 * extra), which counts only from a browser.
 */
class WebSignInActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val web = WebView(this)
        // The page's real address, as an in-app browser opens a link, served
        // from here: the network never sees it, and the page's origin (what
        // autofill is told) is the site's.
        web.webViewClient =
            object : WebViewClient() {
                override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest) =
                    WebResourceResponse("text/html", "utf-8", PAGE.byteInputStream())
            }
        setContentView(web)
        web.loadUrl(intent.getStringExtra("site") ?: "https://elsewhere.test/")
    }

    private companion object {
        // No blinking caret: it keeps the screen from settling, which the test waits for.
        const val PAGE =
            "<!doctype html><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">" +
                "<style>input{display:block;width:85%;font-size:20px;margin:16px;padding:8px;caret-color:transparent}</style>" +
                "<form><input id=\"username\" name=\"username\" autocomplete=\"username\" placeholder=\"Email\" aria-label=\"Email\" required>" +
                "<input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"current-password\" placeholder=\"Password\" aria-label=\"Password\">" +
                "</form>"
    }
}
