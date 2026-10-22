package com.passbundle.app

import android.os.CancellationSignal
import android.service.autofill.AutofillService
import android.service.autofill.FillCallback
import android.service.autofill.FillRequest
import android.service.autofill.SaveCallback
import android.service.autofill.SaveRequest

/**
 * PassBundle as Android's autofill service (docs/phones.md, "Autofill"). It
 * runs in the app's own process, so it shares the unlocked core with the app:
 * when the app is locked, so is autofill.
 */
class PassBundleAutofill : AutofillService() {
    override fun onFillRequest(request: FillRequest, cancellationSignal: CancellationSignal, callback: FillCallback) {
        val structure = request.fillContexts.lastOrNull()?.structure ?: return callback.onSuccess(null)
        val fields = Fields.find(structure)
        // Never PassBundle's own screens.
        if (fields.packageName == packageName) return callback.onSuccess(null)
        callback.onSuccess(Responses.forFields(this, fields))
    }

    override fun onSaveRequest(request: SaveRequest, callback: SaveCallback) {
        // Saving what someone signs in with comes next (docs/phones.md).
        callback.onSuccess()
    }
}
